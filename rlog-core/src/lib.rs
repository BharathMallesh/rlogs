use std::ffi::CStr;
use std::os::raw::c_char;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::panic;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use base64::{engine::general_purpose::STANDARD as BASE64_STD, Engine as _};
use crossbeam_channel::{bounded, Sender};
use lazy_static::lazy_static;
use tracing::{error, info, debug, trace, warn};
use std::thread;
use std::sync::Mutex;
use quick_xml::Reader;
use quick_xml::events::Event;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use chrono::Utc;
use flate2::write::GzEncoder;
use flate2::Compression;

// ─────────────────────────────────────────────────────────────────────────────
// Enums
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum RollingPolicy { Never, Hourly, Daily }

/// Output format, set by the layout element inside each appender in log4j2.xml.
#[derive(Clone, PartialEq)]
enum LogFormat { Text, Json, Xml, Logfmt }

// ─────────────────────────────────────────────────────────────────────────────
// Retention / archival policy
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RetentionConfig {
    max_size_bytes:   u64,
    max_backup_files: u32,
    compress:         bool,
    archive_path:     Option<PathBuf>,
    retention_days:   u64,
}
impl Default for RetentionConfig {
    fn default() -> Self {
        Self { max_size_bytes: 0, max_backup_files: 0, compress: false,
               archive_path: None, retention_days: 0 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HMAC-SHA256 signing config — populated from <LogSigning> in log4j2.xml
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SigningConfig {
    key: Vec<u8>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Global config (written by rlog_configure, consumed once by LOG_SENDER init)
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of everything the background thread needs; cloned cheaply on restart.
#[derive(Clone)]
struct BackgroundConfig {
    file_name:      Option<String>,
    rolling_policy: RollingPolicy,
    level:          tracing::Level,
    has_console:    bool,
    format:         LogFormat,
    retention:      RetentionConfig,
    signing:        Option<SigningConfig>,
    node_id:        String,
}

struct RlogConfig {
    file_name:      Option<String>,
    rolling_policy: RollingPolicy,
    level:          tracing::Level,
    has_console:    bool,
    format:         LogFormat,
    retention:      RetentionConfig,
    signing:        Option<SigningConfig>,
    node_id:        String,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name:      None,
        rolling_policy: RollingPolicy::Never,
        level:          tracing::Level::TRACE,
        has_console:    false,
        format:         LogFormat::Text,
        retention:      RetentionConfig::default(),
        signing:        None,
        node_id:        get_node_id(),
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Channel event
// ─────────────────────────────────────────────────────────────────────────────

enum LogEvent {
    Message { level: i32, message: String, context: Option<String> },
    Flush(crossbeam_channel::Sender<()>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Crash-recovery flag: tracing-subscriber can only be initialised once globally.
// ─────────────────────────────────────────────────────────────────────────────
static TRACING_INIT: AtomicBool = AtomicBool::new(false);
static LOG_SEQUENCE: AtomicU64  = AtomicU64::new(1);

// ─────────────────────────────────────────────────────────────────────────────
// Utility helpers
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_env_expr(s: &str) -> String {
    let mut result = s.to_string();
    loop {
        let start = match result.find("${") { Some(i) => i, None => break };
        let end   = match result[start..].find('}') { Some(i) => start + i, None => break };
        let expr  = &result[start + 2..end];
        let resolved = if let Some(rest) = expr.strip_prefix("env:") {
            let (var, default) = match rest.find(":-") {
                Some(d) => (&rest[..d], &rest[d + 2..]),
                None    => (rest, ""),
            };
            std::env::var(var).unwrap_or_else(|_| default.to_string())
        } else { break };
        result = format!("{}{}{}", &result[..start], resolved, &result[end + 1..]);
    }
    result
}

fn parse_size(s: &str) -> u64 {
    let s     = s.trim();
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let num: f64 = s[..split].trim().parse().unwrap_or(0.0);
    match s[split..].trim().to_uppercase().as_str() {
        "KB" | "KIB" => (num * 1_024.0) as u64,
        "MB" | "MIB" => (num * 1_048_576.0) as u64,
        "GB" | "GIB" => (num * 1_073_741_824.0) as u64,
        _            => num as u64,
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _    => out.push(c),
        }
    }
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&'  => out.push_str("&amp;"),
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _    => out.push(c),
        }
    }
    out
}

fn logfmt_quote(s: &str) -> String {
    if s.chars().any(|c| matches!(c, ' ' | '"' | '=' | '\n' | '\r' | '\t')) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn parse_mdc_json(ctx: &str) -> Vec<(String, String)> {
    fn read_str<'a>(s: &'a str) -> Option<(String, &'a str)> {
        let s = s.trim_start_matches([' ', ',']);
        if !s.starts_with('"') { return None; }
        let mut out   = String::new();
        let mut chars = s[1..].char_indices();
        loop {
            match chars.next() {
                None => return None,
                Some((i, '"'))  => return Some((out, &s[i + 2..])),
                Some((_, '\\')) => match chars.next() {
                    Some((_, '"'))  => out.push('"'),
                    Some((_, '\\')) => out.push('\\'),
                    Some((_, 'n'))  => out.push('\n'),
                    Some((_, 'r'))  => out.push('\r'),
                    Some((_, 't'))  => out.push('\t'),
                    Some((_, c))    => { out.push('\\'); out.push(c); }
                    None            => return None,
                },
                Some((_, c)) => out.push(c),
            }
        }
    }
    let s = ctx.trim().trim_start_matches('{').trim_end_matches('}').trim();
    if s.is_empty() { return vec![]; }
    let mut pairs = Vec::new();
    let mut rest  = s;
    loop {
        rest = rest.trim_start_matches([' ', ',']);
        if rest.is_empty() { break; }
        let (key, ak)  = match read_str(rest)        { Some(v) => v, None => break };
        let ac         = ak.trim_start_matches([' ', ':']);
        let (val, av)  = match read_str(ac)          { Some(v) => v, None => break };
        pairs.push((key, val));
        rest = av;
    }
    pairs
}

fn level_name(level: i32) -> &'static str {
    match level { 1 => "TRACE", 2 => "DEBUG", 3 => "INFO", 4 => "WARN", 5 => "ERROR", _ => "INFO" }
}

fn time_key_now(policy: &RollingPolicy) -> String {
    let now = Utc::now();
    match policy {
        RollingPolicy::Never  => String::new(),
        RollingPolicy::Daily  => now.format("%Y-%m-%d").to_string(),
        RollingPolicy::Hourly => now.format("%Y-%m-%dT%H").to_string(),
    }
}

/// Best-effort node identifier for multi-node log correlation.
/// Priority: NODE_ID env → HOSTNAME env → COMPUTERNAME (Windows) → /etc/hostname → "localhost"
fn get_node_id() -> String {
    for var in &["NODE_ID", "HOSTNAME", "COMPUTERNAME"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if !v.is_empty() { return v; }
        }
    }
    if let Ok(s) = std::fs::read_to_string("/etc/hostname") {
        let s = s.trim().to_string();
        if !s.is_empty() { return s; }
    }
    "localhost".to_string()
}

fn format_panic_msg(e: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<&str>()  { return s.to_string(); }
    if let Some(s) = e.downcast_ref::<String>() { return s.clone(); }
    "unknown panic payload".to_string()
}

fn hmac_sha256_b64(key: &[u8], data: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .expect("HMAC accepts any key size");
    mac.update(data.as_bytes());
    BASE64_STD.encode(mac.finalize().into_bytes())
}

fn load_signing_key(raw: &str) -> Vec<u8> {
    if let Some(hex) = raw.strip_prefix("hex:") {
        (0..hex.len()).step_by(2)
            .filter_map(|i| u8::from_str_radix(hex.get(i..i+2).unwrap_or(""), 16).ok())
            .collect()
    } else if let Some(b64) = raw.strip_prefix("b64:") {
        BASE64_STD.decode(b64.trim()).unwrap_or_else(|_| b64.as_bytes().to_vec())
    } else {
        raw.as_bytes().to_vec()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ManagedRollingFile — size cap + time rolling + gzip + retention + archival
// ─────────────────────────────────────────────────────────────────────────────

struct ManagedRollingFile {
    dir:              PathBuf,
    base_stem:        String,
    base_ext:         String,
    path:             PathBuf,
    writer:           Option<BufWriter<File>>,
    current_size:     u64,
    time_key:         String,
    max_size_bytes:   u64,
    max_backup_files: u32,
    compress:         bool,
    archive_path:     Option<PathBuf>,
    retention_days:   u64,
    rolling_policy:   RollingPolicy,
}

impl ManagedRollingFile {
    fn open(log_path: &Path, r: &RetentionConfig, policy: RollingPolicy) -> io::Result<Self> {
        let dir  = log_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let stem = log_path.file_stem().and_then(|s| s.to_str()).unwrap_or("app").to_string();
        let ext  = log_path.extension().and_then(|s| s.to_str()).unwrap_or("log").to_string();
        fs::create_dir_all(&dir)?;
        let file = fs::OpenOptions::new().create(true).append(true).open(log_path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            dir, base_stem: stem, base_ext: ext,
            path: log_path.to_path_buf(),
            writer: Some(BufWriter::new(file)),
            current_size: size,
            time_key: time_key_now(&policy),
            max_size_bytes:   r.max_size_bytes,
            max_backup_files: r.max_backup_files,
            compress:         r.compress,
            archive_path:     r.archive_path.clone(),
            retention_days:   r.retention_days,
            rolling_policy:   policy,
        })
    }

    fn rolled_name(&self, index: u32, gz: bool) -> String {
        let g = if gz { ".gz" } else { "" };
        if self.time_key.is_empty() {
            format!("{}.{:03}.{}{}", self.base_stem, index, self.base_ext, g)
        } else {
            format!("{}.{}.{:03}.{}{}", self.base_stem, self.time_key, index, self.base_ext, g)
        }
    }

    fn should_roll_time(&self) -> bool {
        self.rolling_policy != RollingPolicy::Never
            && time_key_now(&self.rolling_policy) != self.time_key
    }

    fn roll(&mut self) {
        if let Some(ref mut w) = self.writer { let _ = w.flush(); }
        self.writer = None; // closes the file handle

        let index = (1u32..).find(|&i| {
            !self.dir.join(self.rolled_name(i, false)).exists()
            && !self.dir.join(self.rolled_name(i, true)).exists()
        }).unwrap_or(1);

        let rolled_path = self.dir.join(self.rolled_name(index, false));

        if let Err(e) = fs::rename(&self.path, &rolled_path) {
            eprintln!("rlog: roll rename failed for {:?}: {}", self.path, e);
        } else if self.compress {
            let rp = rolled_path.clone();
            thread::spawn(move || {
                if let Err(e) = compress_file(&rp) {
                    eprintln!("rlog: gzip failed for {:?}: {}", rp, e);
                }
            });
        }

        match fs::OpenOptions::new().create(true).write(true).truncate(false).open(&self.path) {
            Ok(f) => {
                self.writer       = Some(BufWriter::new(f));
                self.current_size = 0;
                self.time_key     = time_key_now(&self.rolling_policy);
            }
            Err(e) => eprintln!("rlog: could not open new log file {:?}: {}", self.path, e),
        }

        let (dir, stem, ext, mb, arch, rd) = (
            self.dir.clone(), self.base_stem.clone(), self.base_ext.clone(),
            self.max_backup_files, self.archive_path.clone(), self.retention_days,
        );
        thread::spawn(move || apply_retention(&dir, &stem, &ext, mb, arch.as_deref(), rd));
    }
}

impl Write for ManagedRollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.should_roll_time() { self.roll(); }
        let n = match self.writer {
            Some(ref mut w) => w.write(buf)?,
            None            => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "log unavailable after failed roll")),
        };
        self.current_size += n as u64;
        if self.max_size_bytes > 0 && self.current_size >= self.max_size_bytes { self.roll(); }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.writer { Some(ref mut w) => w.flush(), None => Ok(()) }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Compression + retention (run in their own background threads)
// ─────────────────────────────────────────────────────────────────────────────

fn compress_file(path: &Path) -> io::Result<()> {
    // Guard: file may have been deleted between roll() and when this thread ran.
    if !path.exists() { return Ok(()); }
    let gz_path = PathBuf::from(format!("{}.gz", path.display()));
    let input   = File::open(path)?;
    let output  = File::create(&gz_path)?;
    let mut enc = GzEncoder::new(output, Compression::default());
    if let Err(e) = io::copy(&mut io::BufReader::new(input), &mut enc) {
        let _ = fs::remove_file(&gz_path); // clean up partial .gz on error
        return Err(e);
    }
    enc.finish()?;
    fs::remove_file(path)
}

fn apply_retention(
    dir: &Path, stem: &str, ext: &str,
    max_backup: u32, archive_path: Option<&Path>, retention_days: u64,
) {
    let prefix = format!("{}.", stem);
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).map(|name| {
                name.starts_with(&prefix)
                && (name.ends_with(&format!(".{}", ext)) || name.ends_with(&format!(".{}.gz", ext)))
                && name != &format!("{}.{}", stem, ext)
            }).unwrap_or(false))
            .collect(),
        Err(_) => return,
    };

    let now          = std::time::SystemTime::now();
    let max_age_secs = retention_days * 86_400;
    let mut live: Vec<PathBuf> = Vec::new();

    for p in entries.drain(..) {
        let age = fs::metadata(&p).and_then(|m| m.modified()).ok()
            .and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs()).unwrap_or(0);

        if retention_days > 0 && age > max_age_secs {
            if let Some(adir) = archive_path {
                let _ = fs::create_dir_all(adir);
                if let Some(name) = p.file_name() {
                    let dest = adir.join(name);
                    if fs::rename(&p, &dest).is_err() { let _ = fs::remove_file(&p); }
                }
            } else {
                let _ = fs::remove_file(&p);
            }
        } else {
            live.push(p);
        }
    }

    if max_backup > 0 && live.len() > max_backup as usize {
        live.sort();
        for p in live.iter().take(live.len() - max_backup as usize) {
            let _ = fs::remove_file(p);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Structured-format output loop (JSON / XML / Logfmt)
// ─────────────────────────────────────────────────────────────────────────────

fn run_structured_loop(
    receiver: crossbeam_channel::Receiver<LogEvent>,
    mut writers: Vec<Box<dyn Write + Send>>,
    format: LogFormat,
    signing: Option<SigningConfig>,
    node_id: String,
) {
    while let Ok(event) = receiver.recv() {
        match event {
            LogEvent::Message { level, message, context } => {
                let ts  = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                let lvl = level_name(level);
                let mdc = context.as_deref().map(parse_mdc_json).unwrap_or_default();
                let seq = signing.as_ref().map(|_| LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed));

                let line: String = match format {
                    LogFormat::Json => {
                        let mut s = format!(
                            "{{\"@timestamp\":\"{ts}\",\"level\":\"{lvl}\",\"nodeId\":\"{}\",\"message\":\"{}\"",
                            json_escape(&node_id),
                            json_escape(&message)
                        );
                        for (k, v) in &mdc { s.push_str(&format!(",\"{}\":\"{}\"", json_escape(k), json_escape(v))); }
                        if let (Some(sc), Some(n)) = (signing.as_ref(), seq) {
                            s.push_str(&format!(",\"_seq\":{}", n));
                            s.push('}');
                            let sig = hmac_sha256_b64(&sc.key, &s);
                            s.pop();
                            s.push_str(&format!(",\"_sig\":\"{}\"}}\n", sig));
                        } else {
                            s.push_str("}\n");
                        }
                        s
                    }
                    LogFormat::Xml => {
                        let mut s = format!(
                            "<event timestamp=\"{ts}\" level=\"{lvl}\" nodeId=\"{}\"><message>{}</message>",
                            xml_escape(&node_id),
                            xml_escape(&message)
                        );
                        if !mdc.is_empty() {
                            s.push_str("<context>");
                            for (k, v) in &mdc { s.push_str(&format!("<entry key=\"{}\" value=\"{}\"/>", xml_escape(k), xml_escape(v))); }
                            s.push_str("</context>");
                        }
                        if let (Some(sc), Some(n)) = (signing.as_ref(), seq) {
                            s.push_str(&format!("<seq>{}</seq>", n));
                            let sig = hmac_sha256_b64(&sc.key, &s);
                            s.push_str(&format!("<sig>{}</sig>", sig));
                        }
                        s.push_str("</event>\n");
                        s
                    }
                    LogFormat::Logfmt => {
                        let mut s = format!("ts={ts} level={lvl} nodeId={} msg={}",
                            logfmt_quote(&node_id),
                            logfmt_quote(&message));
                        for (k, v) in &mdc { s.push_str(&format!(" {}={}", k, logfmt_quote(v))); }
                        if let (Some(sc), Some(n)) = (signing.as_ref(), seq) {
                            s.push_str(&format!(" _seq={}", n));
                            let sig = hmac_sha256_b64(&sc.key, &s);
                            s.push_str(&format!(" _sig={}", sig));
                        }
                        s.push('\n');
                        s
                    }
                    LogFormat::Text => unreachable!(),
                };

                for w in writers.iter_mut() { let _ = w.write_all(line.as_bytes()); }
            }
            LogEvent::Flush(ack) => {
                for w in writers.iter_mut() { let _ = w.flush(); }
                let _ = ack.send(());
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core background-thread logic — extracted so catch_unwind can restart it
// ─────────────────────────────────────────────────────────────────────────────

fn run_background(
    receiver: crossbeam_channel::Receiver<LogEvent>,
    cfg: BackgroundConfig,
    is_restart: bool,
) {
    if cfg.format == LogFormat::Text {
        // ── Console: tracing-subscriber (first start only; fall back to eprintln! on restart) ──
        let use_tracing = cfg.has_console && !is_restart
            && !TRACING_INIT.swap(true, Ordering::SeqCst);

        if use_tracing {
            let lf = tracing_subscriber::filter::LevelFilter::from_level(cfg.level);
            tracing_subscriber::registry()
                .with(lf)
                .with(tracing_subscriber::fmt::layer().with_writer(io::stdout))
                .init();
        }

        // ── File: ManagedRollingFile ──────────────────────────────────────────
        let mut file_writer: Option<ManagedRollingFile> = cfg.file_name.as_ref().and_then(|f| {
            let p = Path::new(f);
            if let Some(d) = p.parent() { let _ = fs::create_dir_all(d); }
            match ManagedRollingFile::open(p, &cfg.retention, cfg.rolling_policy.clone()) {
                Ok(w)  => Some(w),
                Err(e) => { eprintln!("rlog: could not open {:?}: {}", p, e); None }
            }
        });

        while let Ok(event) = receiver.recv() {
            match event {
                LogEvent::Message { level, message, context } => {
                    let mdc = context.as_deref().map(parse_mdc_json).unwrap_or_default();
                    let mut mdc_suffix = String::new();
                    for (k, v) in &mdc { mdc_suffix.push_str(&format!(" {}={}", k, v)); }

                    // Compute signing suffix once; reuse for both console and file outputs.
                    // Canonical form covers level + message + MDC + seq (no timestamp so
                    // both outputs carry an identical, verifiable signature).
                    let sig_suffix = cfg.signing.as_ref().map(|sc| {
                        let seq = LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                        let canonical = format!("{} [{}] {}{} _seq={}", level_name(level), cfg.node_id, message, mdc_suffix, seq);
                        let sig = hmac_sha256_b64(&sc.key, &canonical);
                        format!(" _seq={} _sig={}", seq, sig)
                    });

                    // Console
                    if cfg.has_console {
                        let mut full = format!("[{}] {}", cfg.node_id, message);
                        full.push_str(&mdc_suffix);
                        if let Some(ref ss) = sig_suffix { full.push_str(ss); }

                        if use_tracing {
                            match level {
                                1 => trace!("{}", full), 2 => debug!("{}", full),
                                3 => info!("{}", full),  4 => warn!("{}", full),
                                _ => error!("{}", full),
                            }
                        } else {
                            let ts = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                            eprintln!("{} {:5} {}", ts, level_name(level), full);
                        }
                    }

                    // File
                    if let Some(ref mut fw) = file_writer {
                        let ts  = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                        let lvl = level_name(level);
                        let mut line = format!("{} {:5} [{}] {}{}", ts, lvl, cfg.node_id, message, mdc_suffix);
                        if let Some(ref ss) = sig_suffix { line.push_str(ss); }
                        line.push('\n');
                        let _ = fw.write_all(line.as_bytes());
                    }
                }
                LogEvent::Flush(ack) => {
                    if let Some(ref mut fw) = file_writer { let _ = fw.flush(); }
                    let _ = ack.send(());
                }
            }
        }
    } else {
        // ── Structured mode (JSON / XML / Logfmt) ────────────────────────────
        let mut writers: Vec<Box<dyn Write + Send>> = Vec::new();
        if cfg.has_console { writers.push(Box::new(io::stdout())); }
        if let Some(ref fname) = cfg.file_name {
            let p = Path::new(fname);
            match ManagedRollingFile::open(p, &cfg.retention, cfg.rolling_policy) {
                Ok(w)  => writers.push(Box::new(w)),
                Err(e) => eprintln!("rlog: could not open {:?}: {}", p, e),
            }
        }
        if writers.is_empty() { writers.push(Box::new(io::stdout())); }
        run_structured_loop(receiver, writers, cfg.format, cfg.signing, cfg.node_id);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Background thread with watchdog restart loop
//
// If the processing code panics (bad MDC, I/O error in a codepath that unwraps,
// etc.) the outer loop catches the panic with catch_unwind, logs it to stderr,
// and restarts the inner loop.  The channel stays connected so no messages
// are lost during restarts.  After MAX_RESTARTS consecutive panics the thread
// falls back to draining the channel to stderr until the JVM exits.
// ─────────────────────────────────────────────────────────────────────────────

const MAX_RESTARTS: u32 = 10;

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);

        let cfg = {
            let c = CONFIG.lock().unwrap();
            BackgroundConfig {
                file_name:      c.file_name.clone(),
                rolling_policy: c.rolling_policy.clone(),
                level:          c.level,
                has_console:    c.has_console,
                format:         c.format.clone(),
                retention:      c.retention.clone(),
                signing:        c.signing.clone(),
                node_id:        c.node_id.clone(),
            }
        };

        thread::spawn(move || {
            let mut restart_count: u32 = 0;

            'watchdog: loop {
                let r   = receiver.clone();
                let c   = cfg.clone();
                let is_restart = restart_count > 0;

                let result = panic::catch_unwind(panic::AssertUnwindSafe(move || {
                    run_background(r, c, is_restart);
                }));

                match result {
                    Ok(_) => break 'watchdog, // channel closed — normal JVM exit
                    Err(panic_payload) => {
                        restart_count += 1;
                        let msg = format_panic_msg(&panic_payload);
                        eprintln!(
                            "rlog: CRITICAL — background thread panic #{} ({}). {}",
                            restart_count, msg,
                            if restart_count < MAX_RESTARTS { "Restarting." }
                            else { "Switching to stderr-only fallback." }
                        );

                        if restart_count >= MAX_RESTARTS {
                            // Total fallback: drain remaining messages to stderr
                            while let Ok(ev) = receiver.recv() {
                                match ev {
                                    LogEvent::Message { level, message, context } => {
                                        let ts  = Utc::now()
                                            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                                        let mdc = context.as_deref()
                                            .map(parse_mdc_json).unwrap_or_default();
                                        let mut line = format!("[rlog-fallback] {} {} {}",
                                            ts, level_name(level), message);
                                        for (k, v) in &mdc { line.push_str(&format!(" {}={}", k, v)); }
                                        eprintln!("{}", line);
                                    }
                                    LogEvent::Flush(ack) => { let _ = ack.send(()); }
                                }
                            }
                            break 'watchdog;
                        }
                    }
                }
            }
        });

        sender
    };
}

// ─────────────────────────────────────────────────────────────────────────────
// Public C API
// ─────────────────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rlog_configure(xml_ptr: *const c_char) -> i32 {
    if xml_ptr.is_null() { return -1; }
    let c_str = unsafe { CStr::from_ptr(xml_ptr) };
    let Ok(xml_str) = c_str.to_str() else { return -1; };

    let mut reader = Reader::from_str(xml_str);
    reader.trim_text(true);
    let mut config = CONFIG.lock().unwrap();
    let mut buf    = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();
                match tag.as_str() {
                    "jsonlayout"   => { config.format = LogFormat::Json; }
                    "xmllayout"    => { config.format = LogFormat::Xml; }
                    "logfmtlayout" => { config.format = LogFormat::Logfmt; }
                    "console"      => { config.has_console = true; }
                    "file" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"fileName" {
                                if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                    config.file_name      = Some(resolve_env_expr(&raw));
                                    config.rolling_policy = RollingPolicy::Never;
                                }
                            }
                        }
                    }
                    "rollingfile" => {
                        let mut fname = None; let mut policy = RollingPolicy::Never;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"fileName" => {
                                    if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                        fname = Some(resolve_env_expr(&raw));
                                    }
                                }
                                b"filePattern" => {
                                    if let Ok(p) = String::from_utf8(attr.value.into_owned()) {
                                        policy = if p.contains("HH") { RollingPolicy::Hourly }
                                                 else if p.contains("%d{") { RollingPolicy::Daily }
                                                 else { RollingPolicy::Never };
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(f) = fname { config.file_name = Some(f); config.rolling_policy = policy; }
                    }
                    "sizebasedtriggeringpolicy" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"size" {
                                if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                    config.retention.max_size_bytes = parse_size(&s);
                                }
                            }
                        }
                    }
                    "defaultrolloverstrategy" => {
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"max" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        config.retention.max_backup_files = s.trim().parse().unwrap_or(0);
                                    }
                                }
                                b"compress" | b"compressionlevel" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        let v = s.trim();
                                        config.retention.compress = v != "0" && v.to_lowercase() != "false";
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "retentionpolicy" => {
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"retentionDays" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        config.retention.retention_days = s.trim().parse().unwrap_or(0);
                                    }
                                }
                                b"archivePath" => {
                                    if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                        let r = resolve_env_expr(&raw);
                                        if !r.is_empty() { config.retention.archive_path = Some(PathBuf::from(r)); }
                                    }
                                }
                                b"compress" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        let v = s.trim();
                                        config.retention.compress = v != "0" && v.to_lowercase() != "false";
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "root" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"level" {
                                if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                    config.level = match s.to_uppercase().as_str() {
                                        "TRACE" => tracing::Level::TRACE, "DEBUG" => tracing::Level::DEBUG,
                                        "INFO"  => tracing::Level::INFO,  "WARN"  => tracing::Level::WARN,
                                        "ERROR" | "FATAL" => tracing::Level::ERROR, _ => tracing::Level::INFO,
                                    };
                                }
                            }
                        }
                    }
                    "nodeid" => {
                        // <NodeId value="payment-node-1"/>
                        // <NodeId env="NODE_ID" default="node0"/>
                        let mut value_attr:   Option<String> = None;
                        let mut env_attr:     Option<String> = None;
                        let mut default_attr: Option<String> = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"value" => {
                                    if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                        value_attr = Some(v);
                                    }
                                }
                                b"env" => {
                                    if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                        env_attr = Some(v);
                                    }
                                }
                                b"default" => {
                                    if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                        default_attr = Some(v);
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(v) = value_attr {
                            config.node_id = v;
                        } else if let Some(var) = env_attr {
                            config.node_id = std::env::var(var.trim())
                                .unwrap_or_else(|_| default_attr.unwrap_or_else(|| config.node_id.clone()));
                        } else if let Some(d) = default_attr {
                            config.node_id = d;
                        }
                        // No attributes → keep auto-detected hostname
                    }
                    "logsigning" => {
                        let mut key_bytes: Option<Vec<u8>> = None;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"keyEnv" => {
                                    if let Ok(var) = String::from_utf8(attr.value.into_owned()) {
                                        if let Ok(val) = std::env::var(var.trim()) {
                                            key_bytes = Some(load_signing_key(&val));
                                        }
                                    }
                                }
                                b"keyFile" => {
                                    if let Ok(path) = String::from_utf8(attr.value.into_owned()) {
                                        if let Ok(content) = fs::read_to_string(path.trim()) {
                                            key_bytes = Some(load_signing_key(content.trim()));
                                        }
                                    }
                                }
                                b"keyHex" => {
                                    if let Ok(hex) = String::from_utf8(attr.value.into_owned()) {
                                        key_bytes = Some(load_signing_key(&format!("hex:{}", hex.trim())));
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(k) = key_bytes {
                            config.signing = Some(SigningConfig { key: k });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    0
}

#[no_mangle]
pub extern "C" fn rlog_init() -> i32 { let _ = LOG_SENDER.len(); 0 }

#[no_mangle]
pub extern "C" fn rlog_log(level: i32, msg_ptr: *const c_char) {
    rlog_log_with_context(level, msg_ptr, std::ptr::null());
}

#[no_mangle]
pub extern "C" fn rlog_log_with_context(level: i32, msg_ptr: *const c_char, ctx_ptr: *const c_char) {
    if msg_ptr.is_null() { return; }
    let c_str = unsafe { CStr::from_ptr(msg_ptr) };
    if let Ok(str_slice) = c_str.to_str() {
        let context = if !ctx_ptr.is_null() {
            unsafe { CStr::from_ptr(ctx_ptr) }.to_str().ok().map(|s| s.to_owned())
        } else { None };
        if let Err(e) = LOG_SENDER.send(LogEvent::Message { level, message: str_slice.to_owned(), context }) {
            if let LogEvent::Message { level, message, .. } = e.into_inner() {
                eprintln!("rlog: CRITICAL [{}] {} — channel dead, wrote to stderr", level_name(level), message);
            }
        }
    }
}

/// Block until every message enqueued before this call has been written to all
/// outputs (or until the 5-second watchdog timeout).  Called automatically from
/// the JVM shutdown hook registered in NativeLogger's static initialiser.
#[no_mangle]
pub extern "C" fn rlog_flush() {
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    let _ = LOG_SENDER.send(LogEvent::Flush(ack_tx));
    let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(5));
}

// ─────────────────────────────────────────────────────────────────────────────
// JNI entry points — thin wrappers used by Java 8–21 (before FFM is available).
// Java 22+ uses the FFM path in NativeLogger.java instead.
// ─────────────────────────────────────────────────────────────────────────────

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jint;

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLoggerJNI_rlogConfigure<'local>(
    mut env: JNIEnv<'local>, _class: JClass<'local>, xml: JString<'local>,
) -> jint {
    match env.get_string(&xml) {
        Ok(s) => {
            let xml_string: String = s.into();
            match std::ffi::CString::new(xml_string) {
                Ok(c) => rlog_configure(c.as_ptr()),
                Err(_) => -1,
            }
        }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLoggerJNI_rlogInit<'local>(
    _env: JNIEnv<'local>, _class: JClass<'local>,
) -> jint {
    rlog_init()
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLoggerJNI_rlogLog<'local>(
    mut env: JNIEnv<'local>, _class: JClass<'local>,
    level: jint, message: JString<'local>, mdc: JString<'local>,
) {
    let msg: String = match env.get_string(&message) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    // mdc may be null (Java passes null when ThreadContext is empty)
    let context: Option<String> = if mdc.is_null() {
        None
    } else {
        env.get_string(&mdc).ok().map(|s| s.into())
    };
    if let Err(e) = LOG_SENDER.send(LogEvent::Message { level, message: msg, context }) {
        if let LogEvent::Message { level, message, .. } = e.into_inner() {
            eprintln!("rlog: CRITICAL [{}] {} — channel dead, wrote to stderr", level_name(level), message);
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLoggerJNI_rlogFlush<'local>(
    _env: JNIEnv<'local>, _class: JClass<'local>,
) {
    rlog_flush();
}
