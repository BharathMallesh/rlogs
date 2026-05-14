use std::ffi::CStr;
use std::os::raw::c_char;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
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

/// Output format selected by the layout child element inside each appender.
///
/// | log4j2.xml element    | Format   | Typical use                                   |
/// |-----------------------|----------|-----------------------------------------------|
/// | `<PatternLayout/>`    | Text     | Development / ops consoles                    |
/// | `<JsonLayout/>`       | Json     | SIEM / Elasticsearch / Splunk (ECS-compatible)|
/// | `<XmlLayout/>`        | Xml      | Legacy XML pipelines, Log4j2 chain             |
/// | `<LogfmtLayout/>`     | Logfmt   | Loki / Datadog / Grafana Agent                |
#[derive(Clone, PartialEq)]
enum LogFormat { Text, Json, Xml, Logfmt }

// ─────────────────────────────────────────────────────────────────────────────
// Retention & archival policy (populated from XML before rlog_init)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RetentionConfig {
    /// Roll file when it reaches this size.  0 = no size limit.
    max_size_bytes:   u64,
    /// Keep at most this many rolled files per directory.  0 = unlimited.
    max_backup_files: u32,
    /// Gzip-compress each rolled file after rotation.
    compress:         bool,
    /// Move expired files here instead of deleting them (cold-storage / NAS).
    archive_path:     Option<PathBuf>,
    /// Delete (or archive) rolled files older than this many days.  0 = keep forever.
    /// NPCI / PCI-DSS typically mandate 7 years = 2555 days.
    retention_days:   u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self { max_size_bytes: 0, max_backup_files: 0, compress: false,
               archive_path: None, retention_days: 0 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Global config (written once by rlog_configure, read once by LOG_SENDER init)
// ─────────────────────────────────────────────────────────────────────────────

struct RlogConfig {
    file_name:      Option<String>,
    rolling_policy: RollingPolicy,
    level:          tracing::Level,
    has_console:    bool,
    format:         LogFormat,
    retention:      RetentionConfig,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name:      None,
        rolling_policy: RollingPolicy::Never,
        level:          tracing::Level::TRACE,
        has_console:    false,
        format:         LogFormat::Text,
        retention:      RetentionConfig::default(),
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Channel event
// ─────────────────────────────────────────────────────────────────────────────

enum LogEvent {
    Message { level: i32, message: String, context: Option<String> },
    /// Drain sentinel — ack on the enclosed one-shot channel when all prior
    /// messages have been written.  Used by rlog_flush / JVM shutdown hook.
    Flush(crossbeam_channel::Sender<()>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility: env-expression resolver  ${env:VAR:-default}
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
        } else {
            break;
        };
        result = format!("{}{}{}", &result[..start], resolved, &result[end + 1..]);
    }
    result
}

/// Parse a human-readable size string: "100 MB", "512KB", "1GiB", etc.
fn parse_size(s: &str) -> u64 {
    let s       = s.trim();
    let split   = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let num: f64 = s[..split].trim().parse().unwrap_or(0.0);
    match s[split..].trim().to_uppercase().as_str() {
        "KB" | "KIB" => (num * 1_024.0) as u64,
        "MB" | "MIB" => (num * 1_048_576.0) as u64,
        "GB" | "GIB" => (num * 1_073_741_824.0) as u64,
        _            => num as u64,   // bare number or "B" → bytes
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Utility: output formatters
// ─────────────────────────────────────────────────────────────────────────────

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

/// Parse the JSON MDC object produced by `RlogLogger.mdcToJson()`.
/// Input: `{"txnId":"abc123","userId":"xyz"}`  →  `[("txnId","abc123"),…]`
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
        let (key, after_key)   = match read_str(rest)     { Some(v) => v, None => break };
        let after_colon        = after_key.trim_start_matches([' ', ':']);
        let (val, after_val)   = match read_str(after_colon) { Some(v) => v, None => break };
        pairs.push((key, val));
        rest = after_val;
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

// ─────────────────────────────────────────────────────────────────────────────
// Managed rolling file writer
// ─────────────────────────────────────────────────────────────────────────────

/// A `Write` implementation that handles size-based rolling, optional gzip
/// compression, time-based rolling, max-backup-count enforcement, archival,
/// and retention-day expiry.  Used for all file output.
struct ManagedRollingFile {
    dir:              PathBuf,
    base_stem:        String,          // "app" extracted from "app.log"
    base_ext:         String,          // "log"
    path:             PathBuf,         // dir/app.log  (current active file)
    writer:           Option<BufWriter<File>>,
    current_size:     u64,
    time_key:         String,          // e.g. "2026-05-14" for Daily

    max_size_bytes:   u64,
    max_backup_files: u32,
    compress:         bool,
    archive_path:     Option<PathBuf>,
    retention_days:   u64,
    rolling_policy:   RollingPolicy,
}

impl ManagedRollingFile {
    fn open(
        log_path: &Path,
        retention: &RetentionConfig,
        rolling_policy: RollingPolicy,
    ) -> io::Result<Self> {
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
            time_key: time_key_now(&rolling_policy),
            max_size_bytes:   retention.max_size_bytes,
            max_backup_files: retention.max_backup_files,
            compress:         retention.compress,
            archive_path:     retention.archive_path.clone(),
            retention_days:   retention.retention_days,
            rolling_policy,
        })
    }

    fn rolled_name(&self, index: u32, gz: bool) -> String {
        let gz_sfx = if gz { ".gz" } else { "" };
        if self.time_key.is_empty() {
            format!("{}.{:03}.{}{}", self.base_stem, index, self.base_ext, gz_sfx)
        } else {
            format!("{}.{}.{:03}.{}{}", self.base_stem, self.time_key, index, self.base_ext, gz_sfx)
        }
    }

    fn should_roll_time(&self) -> bool {
        if self.rolling_policy == RollingPolicy::Never { return false; }
        time_key_now(&self.rolling_policy) != self.time_key
    }

    fn roll(&mut self) {
        // 1. Flush + close current file (drop the writer)
        if let Some(ref mut w) = self.writer { let _ = w.flush(); }
        self.writer = None;

        // 2. Find next available rolled-file index
        let index = (1u32..).find(|&i| {
            !self.dir.join(self.rolled_name(i, false)).exists()
            && !self.dir.join(self.rolled_name(i, true)).exists()
        }).unwrap_or(1);

        let rolled_path = self.dir.join(self.rolled_name(index, false));

        if let Err(e) = fs::rename(&self.path, &rolled_path) {
            eprintln!("rlog: roll rename failed for {:?}: {}", self.path, e);
        } else if self.compress {
            // 3. Compress asynchronously so the logging thread is not blocked
            let rp = rolled_path.clone();
            thread::spawn(move || {
                if let Err(e) = compress_file(&rp) {
                    eprintln!("rlog: compression failed for {:?}: {}", rp, e);
                }
            });
        }

        // 4. Open fresh active log file
        match fs::OpenOptions::new().create(true).write(true).truncate(false).open(&self.path) {
            Ok(f) => {
                self.writer       = Some(BufWriter::new(f));
                self.current_size = 0;
                self.time_key     = time_key_now(&self.rolling_policy);
            }
            Err(e) => eprintln!("rlog: could not open new log file {:?}: {}", self.path, e),
        }

        // 5. Enforce retention + archival asynchronously
        let dir          = self.dir.clone();
        let stem         = self.base_stem.clone();
        let ext          = self.base_ext.clone();
        let max_backup   = self.max_backup_files;
        let archive      = self.archive_path.clone();
        let ret_days     = self.retention_days;
        thread::spawn(move || {
            apply_retention(&dir, &stem, &ext, max_backup, archive.as_deref(), ret_days);
        });
    }
}

impl Write for ManagedRollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Time-based roll check (e.g. midnight crossed between messages)
        if self.should_roll_time() { self.roll(); }

        let n = match self.writer {
            Some(ref mut w) => w.write(buf)?,
            None            => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "log file unavailable after failed roll")),
        };
        self.current_size += n as u64;

        // Size-based roll after writing (current message is already in the file)
        if self.max_size_bytes > 0 && self.current_size >= self.max_size_bytes {
            self.roll();
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.writer { Some(ref mut w) => w.flush(), None => Ok(()) }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Compression + retention helpers (run in their own threads)
// ─────────────────────────────────────────────────────────────────────────────

/// Gzip-compress `path` → `path.gz`, then delete the original.
/// Uses streaming io::copy so memory usage stays bounded regardless of file size.
fn compress_file(path: &Path) -> io::Result<()> {
    let gz_path = PathBuf::from(format!("{}.gz", path.display()));
    let input   = File::open(path)?;
    let output  = File::create(&gz_path)?;
    let mut enc = GzEncoder::new(output, Compression::default());
    io::copy(&mut io::BufReader::new(input), &mut enc)?;
    enc.finish()?;
    fs::remove_file(path)
}

/// Enforce max-backup-file count, archival, and retention-day expiry for
/// all rolled log files in `dir` whose names start with `stem.` and end with
/// `.ext` or `.ext.gz`.
fn apply_retention(
    dir:          &Path,
    stem:         &str,
    ext:          &str,
    max_backup:   u32,
    archive_path: Option<&Path>,
    retention_days: u64,
) {
    let prefix = format!("{}.", stem);
    let mut entries: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).map(|name| {
                    name.starts_with(&prefix)
                    && (name.ends_with(&format!(".{}", ext))
                        || name.ends_with(&format!(".{}.gz", ext)))
                    // Exclude the live file "stem.ext"
                    && name != &format!("{}.{}", stem, ext)
                }).unwrap_or(false)
            })
            .collect(),
        Err(_) => return,
    };

    let now          = std::time::SystemTime::now();
    let max_age_secs = retention_days * 86_400;
    let mut live: Vec<PathBuf> = Vec::new();

    // ── Step 1: archive / delete files that have exceeded retention_days ──
    for p in entries.drain(..) {
        let age_secs = fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if retention_days > 0 && age_secs > max_age_secs {
            if let Some(adir) = archive_path {
                let _ = fs::create_dir_all(adir);
                if let Some(name) = p.file_name() {
                    let dest = adir.join(name);
                    if let Err(e) = fs::rename(&p, &dest) {
                        eprintln!("rlog: archive move failed for {:?} ({}), deleting", p, e);
                        let _ = fs::remove_file(&p);
                    }
                }
            } else {
                let _ = fs::remove_file(&p);
            }
        } else {
            live.push(p);
        }
    }

    // ── Step 2: enforce max_backup_files on the remaining live files ──
    if max_backup > 0 && live.len() > max_backup as usize {
        live.sort(); // date-timestamped names sort oldest-first lexicographically
        let excess = live.len() - max_backup as usize;
        for p in live.iter().take(excess) {
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
) {
    while let Ok(event) = receiver.recv() {
        match event {
            LogEvent::Message { level, message, context } => {
                let ts  = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                let lvl = level_name(level);
                let mdc = context.as_deref().map(parse_mdc_json).unwrap_or_default();

                let line: String = match format {
                    // ── JSON ──────────────────────────────────────────────
                    // {"@timestamp":"…","level":"INFO","message":"…","txnId":"abc"}
                    LogFormat::Json => {
                        let mut s = format!(
                            "{{\"@timestamp\":\"{ts}\",\"level\":\"{lvl}\",\"message\":\"{}\"",
                            json_escape(&message)
                        );
                        for (k, v) in &mdc {
                            s.push_str(&format!(",\"{}\":\"{}\"", json_escape(k), json_escape(v)));
                        }
                        s.push_str("}\n");
                        s
                    }

                    // ── XML ───────────────────────────────────────────────
                    // <event timestamp="…" level="INFO"><message>…</message>
                    //   <context><entry key="txnId" value="abc"/></context></event>
                    LogFormat::Xml => {
                        let mut s = format!(
                            "<event timestamp=\"{ts}\" level=\"{lvl}\"><message>{}</message>",
                            xml_escape(&message)
                        );
                        if !mdc.is_empty() {
                            s.push_str("<context>");
                            for (k, v) in &mdc {
                                s.push_str(&format!(
                                    "<entry key=\"{}\" value=\"{}\"/>",
                                    xml_escape(k), xml_escape(v)
                                ));
                            }
                            s.push_str("</context>");
                        }
                        s.push_str("</event>\n");
                        s
                    }

                    // ── Logfmt ────────────────────────────────────────────
                    // ts=… level=INFO msg="Payment OK" txnId=abc userId=xyz
                    LogFormat::Logfmt => {
                        let mut s = format!(
                            "ts={ts} level={lvl} msg={}", logfmt_quote(&message)
                        );
                        for (k, v) in &mdc {
                            s.push_str(&format!(" {}={}", k, logfmt_quote(v)));
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
// Background thread + lock-free 1M-capacity sender
// ─────────────────────────────────────────────────────────────────────────────

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);

        let (file_name, level, rolling_policy, has_console, format, retention) = {
            let c = CONFIG.lock().unwrap();
            (c.file_name.clone(), c.level, c.rolling_policy.clone(),
             c.has_console, c.format.clone(), c.retention.clone())
        };

        thread::spawn(move || {
            if format == LogFormat::Text {
                // ── Text mode ────────────────────────────────────────────────
                // Console output: tracing-subscriber (pretty timestamps + colours).
                // File output:    ManagedRollingFile with ISO-8601 text lines.
                if has_console {
                    let level_filter = tracing_subscriber::filter::LevelFilter::from_level(level);
                    tracing_subscriber::registry()
                        .with(level_filter)
                        .with(tracing_subscriber::fmt::layer().with_writer(io::stdout))
                        .init();
                }

                let mut file_writer: Option<ManagedRollingFile> = file_name.as_ref()
                    .and_then(|fname| {
                        let p = Path::new(fname);
                        if let Some(dir) = p.parent() { let _ = fs::create_dir_all(dir); }
                        match ManagedRollingFile::open(p, &retention, rolling_policy.clone()) {
                            Ok(w)  => Some(w),
                            Err(e) => { eprintln!("rlog: could not open log file {:?}: {}", p, e); None }
                        }
                    });

                while let Ok(event) = receiver.recv() {
                    match event {
                        LogEvent::Message { level, message, context } => {
                            // Console via tracing macros → tracing-subscriber
                            if has_console {
                                let full = match context.as_deref().map(parse_mdc_json) {
                                    Some(ref mdc) if !mdc.is_empty() => {
                                        let mut s = message.clone();
                                        for (k, v) in mdc { s.push_str(&format!(" {}={}", k, v)); }
                                        s
                                    }
                                    _ => message.clone(),
                                };
                                match level {
                                    1 => trace!("{}", full),
                                    2 => debug!("{}", full),
                                    3 => info!("{}", full),
                                    4 => warn!("{}", full),
                                    5 => error!("{}", full),
                                    _ => info!("{}", full),
                                }
                            }

                            // File via ManagedRollingFile
                            if let Some(ref mut fw) = file_writer {
                                let ts  = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                                let lvl = level_name(level);
                                let mdc = context.as_deref().map(parse_mdc_json).unwrap_or_default();
                                let mut line = format!("{} {:5} {}", ts, lvl, message);
                                for (k, v) in &mdc { line.push_str(&format!(" {}={}", k, v)); }
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
                // ── Structured mode (JSON / XML / Logfmt) ────────────────────
                // All output via direct io::Write — tracing-subscriber not involved.
                let mut writers: Vec<Box<dyn Write + Send>> = Vec::new();

                if has_console { writers.push(Box::new(io::stdout())); }

                if let Some(ref fname) = file_name {
                    let p = Path::new(fname);
                    match ManagedRollingFile::open(p, &retention, rolling_policy) {
                        Ok(w)  => writers.push(Box::new(w)),
                        Err(e) => eprintln!("rlog: could not open log file {:?}: {}", p, e),
                    }
                }

                if writers.is_empty() { writers.push(Box::new(io::stdout())); }

                run_structured_loop(receiver, writers, format);
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

                    // ── Layout selection ─────────────────────────────────────
                    "jsonlayout"   => { config.format = LogFormat::Json; }
                    "xmllayout"    => { config.format = LogFormat::Xml; }
                    "logfmtlayout" => { config.format = LogFormat::Logfmt; }
                    // <PatternLayout/> or absent → LogFormat::Text (default)

                    // ── Appender types ───────────────────────────────────────
                    "console" => { config.has_console = true; }

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
                        let mut fname  = None;
                        let mut policy = RollingPolicy::Never;
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
                        if let Some(f) = fname {
                            config.file_name      = Some(f);
                            config.rolling_policy = policy;
                        }
                    }

                    // ── Retention / archival policy ──────────────────────────
                    //
                    //   <SizeBasedTriggeringPolicy size="100 MB"/>
                    //     → roll file when it reaches this size
                    //
                    //   <DefaultRolloverStrategy max="30" compress="true"/>
                    //     → keep at most 30 rolled files; gzip each rolled file
                    //
                    //   <RetentionPolicy retentionDays="2555"
                    //                    archivePath="${env:ARCHIVE_DIR:-/cold/logs}"
                    //                    compress="true"/>
                    //     → delete/archive files older than 2555 days (≈7 years)

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
                                        config.retention.max_backup_files =
                                            s.trim().parse().unwrap_or(0);
                                    }
                                }
                                b"compress" | b"compressionLevel" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        let v = s.trim();
                                        config.retention.compress =
                                            v != "0" && v.to_lowercase() != "false";
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
                                        config.retention.retention_days =
                                            s.trim().parse().unwrap_or(0);
                                    }
                                }
                                b"archivePath" => {
                                    if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                        let resolved = resolve_env_expr(&raw);
                                        if !resolved.is_empty() {
                                            config.retention.archive_path =
                                                Some(PathBuf::from(resolved));
                                        }
                                    }
                                }
                                b"compress" => {
                                    if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                        let v = s.trim();
                                        config.retention.compress =
                                            v != "0" && v.to_lowercase() != "false";
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    // ── Root logger level ─────────────────────────────────────
                    "root" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"level" {
                                if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                    config.level = match s.to_uppercase().as_str() {
                                        "TRACE"           => tracing::Level::TRACE,
                                        "DEBUG"           => tracing::Level::DEBUG,
                                        "INFO"            => tracing::Level::INFO,
                                        "WARN"            => tracing::Level::WARN,
                                        "ERROR" | "FATAL" => tracing::Level::ERROR,
                                        _                 => tracing::Level::INFO,
                                    };
                                }
                            }
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
pub extern "C" fn rlog_init() -> i32 {
    let _ = LOG_SENDER.len();
    0
}

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
        } else {
            None
        };
        if let Err(e) = LOG_SENDER.send(LogEvent::Message {
            level, message: str_slice.to_owned(), context,
        }) {
            if let LogEvent::Message { message, .. } = e.into_inner() {
                eprintln!("rlog: CRITICAL - log channel closed, message dropped: {}", message);
            }
        }
    }
}

/// Block until every message enqueued before this call has been written to all
/// outputs, or until the 5-second timeout expires.  Called automatically from
/// the JVM shutdown hook registered in NativeLogger's static initialiser.
#[no_mangle]
pub extern "C" fn rlog_flush() {
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    let _ = LOG_SENDER.send(LogEvent::Flush(ack_tx));
    let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(5));
}
