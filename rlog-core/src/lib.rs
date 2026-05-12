use std::ffi::CStr;
use std::os::raw::c_char;
use crossbeam_channel::{bounded, Sender};
use lazy_static::lazy_static;
use std::thread;
use std::sync::Mutex;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use chrono::{DateTime, Utc};
use std::io::Write;

// Queue backpressure policy: 0 = Discard, 1 = Block
static QUEUE_POLICY: AtomicU8 = AtomicU8::new(0);

// Set to true once rlog_init() has been called (background thread is running)
static INITIALIZED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, PartialEq)]
enum RollingPolicy { Never, Hourly, Daily }

#[derive(Clone, PartialEq)]
enum LayoutType { Pattern, Json }

#[derive(Clone)]
enum LogFilter {
    Threshold { min_level: i32 },
    Regex { pattern: regex::Regex, deny_on_match: bool },
}

/// Per-logger configuration entry (mirrors <Logger name="..." level="..." additivity="..."/>)
#[derive(Clone)]
struct PerLoggerConfig {
    name: String,
    min_level: i32,
    additivity: bool,
}

struct RlogConfig {
    file_name: Option<String>,
    rolling_policy: RollingPolicy,
    min_level: i32,       // root level: 1=TRACE 2=DEBUG 3=INFO 4=WARN 5=ERROR
    layout_type: LayoutType,
    max_size: Option<u64>,
    max_files: usize,
    filters: Vec<LogFilter>,
    pattern: String,
    console_enabled: bool,
    loggers: Vec<PerLoggerConfig>,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name: None,
        rolling_policy: RollingPolicy::Never,
        min_level: 1,
        layout_type: LayoutType::Pattern,
        max_size: None,
        max_files: 7,
        filters: Vec::new(),
        pattern: "%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex".to_string(),
        console_enabled: false,
        loggers: Vec::new(),
    });
}

struct LogEvent {
    level: i32,
    message: String,
    mdc: Option<String>,        // MDC flat map  e.g. "{requestId=abc, userId=42}"
    ndc: Option<String>,        // NDC stack     e.g. "outer inner"
    logger_name: Option<String>,
    thread_name: Option<String>,
    exception: Option<String>,  // full stack trace from printStackTrace()
    timestamp: DateTime<Utc>,
}

enum ChannelMessage {
    Log(LogEvent),
    Reconfigure,
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn level_name(level: i32) -> &'static str {
    match level {
        1 => "TRACE",
        2 => "DEBUG",
        3 => "INFO",
        4 => "WARN",
        _ => "ERROR",
    }
}

/// Convert a Java SimpleDateFormat pattern to a formatted timestamp string.
/// Handles the most common tokens: y M d H h m s S E a z Z X plus quoted literals.
fn format_timestamp(ts: &DateTime<Utc>, java_format: &str) -> String {
    if java_format.is_empty() || java_format == "ISO8601" || java_format == "DEFAULT" {
        return ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    }
    let millis = ts.timestamp_subsec_millis();
    let mut result = String::with_capacity(java_format.len() + 10);
    let chars: Vec<char> = java_format.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        // Quoted literal: 'T' or 'text'
        if chars[i] == '\'' {
            i += 1;
            while i < n && chars[i] != '\'' {
                result.push(chars[i]);
                i += 1;
            }
            if i < n { i += 1; }
            continue;
        }

        let ch = chars[i];
        if !ch.is_ascii_alphabetic() {
            result.push(ch);
            i += 1;
            continue;
        }

        // Count consecutive identical characters (e.g. "MM" vs "m")
        let mut count = 0usize;
        while i + count < n && chars[i + count] == ch { count += 1; }
        i += count;

        let segment = match ch {
            'y' => ts.format("%Y").to_string(),
            'M' => if count >= 3 { ts.format("%b").to_string() }
                   else { ts.format("%m").to_string() },
            'd' => ts.format("%d").to_string(),
            'H' => ts.format("%H").to_string(),
            'h' => ts.format("%I").to_string(),
            'm' => ts.format("%M").to_string(),   // minute (lowercase m)
            's' => ts.format("%S").to_string(),
            'S' => {
                let s = format!("{:03}", millis);
                s[..count.min(3)].to_string()
            },
            'a' => ts.format("%p").to_string(),
            'E' => if count >= 4 { ts.format("%A").to_string() }
                   else { ts.format("%a").to_string() },
            'z' | 'Z' | 'X' => ts.format("%z").to_string(),
            _ => std::iter::repeat(ch).take(count).collect(),
        };
        result.push_str(&segment);
    }
    result
}

/// Abbreviate a fully-qualified class name to at most `max_len` characters,
/// shortening package segments from the left (same behaviour as Log4j2).
fn format_logger_name(name: &str, precision: Option<&str>) -> String {
    let prec = match precision {
        None => return name.to_string(),
        Some(p) => p,
    };
    if let Ok(max_len) = prec.parse::<usize>() {
        if name.len() <= max_len { return name.to_string(); }
        let parts: Vec<&str> = name.split('.').collect();
        if parts.len() <= 1 { return name.to_string(); }
        let class = parts[parts.len() - 1];
        let mut abbrev = String::new();
        for (idx, part) in parts.iter().enumerate() {
            if !abbrev.is_empty() { abbrev.push('.'); }
            if idx == parts.len() - 1 {
                abbrev.push_str(part);
            } else {
                abbrev.push(part.chars().next().unwrap_or('?'));
            }
        }
        if abbrev.len() <= max_len { abbrev } else { class.to_string() }
    } else {
        name.to_string()
    }
}

/// Render MDC from a "{k1=v1, k2=v2}" string.
/// With a key arg returns just that value; without returns the full map string.
fn format_mdc(mdc: Option<&str>, key: Option<&str>) -> String {
    match (mdc, key) {
        (None, _) | (Some(""), _) => String::new(),
        (Some(m), None) => m.to_string(),
        (Some(m), Some(k)) => {
            let inner = m.trim_matches(|c| c == '{' || c == '}');
            for pair in inner.split(", ") {
                let mut kv = pair.splitn(2, '=');
                if let (Some(pk), Some(pv)) = (kv.next(), kv.next()) {
                    if pk.trim() == k { return pv.trim().to_string(); }
                }
            }
            String::new()
        }
    }
}

/// Render NDC stack (space-separated values). Optional depth arg limits to last N items.
fn format_ndc(ndc: Option<&str>, depth: Option<&str>) -> String {
    match ndc {
        None | Some("") => String::new(),
        Some(s) => {
            if let Some(d) = depth.and_then(|d| d.parse::<usize>().ok()) {
                let parts: Vec<&str> = s.split(' ').collect();
                if d >= parts.len() { return s.to_string(); }
                parts[parts.len() - d..].join(" ")
            } else {
                s.to_string()
            }
        }
    }
}

/// Apply a Log4j2-style PatternLayout pattern to a log event.
///
/// Supported specifiers:
///   %d{fmt}   timestamp          %t / %thread   thread name
///   %p / %level / %le  level     %c / %logger{N}  logger name
///   %m / %msg  message           %n  newline
///   %X / %X{key}  MDC           %highlight{pat}  ANSI colour
///   %%  literal percent          %-Np  left-justified with min width N
///   %N.Mp  min width N, max (truncate) width M
fn apply_pattern(pattern: &str, event: &LogEvent) -> String {
    let mut result = String::with_capacity(256);
    let bytes = pattern.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] != b'%' {
            result.push(bytes[i] as char);
            i += 1;
            continue;
        }
        i += 1;
        if i >= len { break; }

        // %% → literal %
        if bytes[i] == b'%' {
            result.push('%');
            i += 1;
            continue;
        }

        // Optional left-justify flag
        let left_justify = if i < len && bytes[i] == b'-' { i += 1; true } else { false };

        // Min width
        let mut min_width = 0usize;
        while i < len && bytes[i].is_ascii_digit() {
            min_width = min_width * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }

        // Max width: .N
        let mut max_width: Option<usize> = None;
        if i < len && bytes[i] == b'.' {
            i += 1;
            let mut mw = 0usize;
            while i < len && bytes[i].is_ascii_digit() {
                mw = mw * 10 + (bytes[i] - b'0') as usize;
                i += 1;
            }
            if mw > 0 { max_width = Some(mw); }
        }

        // Specifier keyword (alphabetic)
        let spec_start = i;
        while i < len && bytes[i].is_ascii_alphabetic() { i += 1; }
        let specifier = &pattern[spec_start..i];

        // Optional {arg}  — handles nested braces (e.g. %highlight{%p})
        let mut arg: Option<&str> = None;
        if i < len && bytes[i] == b'{' {
            i += 1;
            let arg_start = i;
            let mut depth = 1usize;
            while i < len {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => { depth -= 1; if depth == 0 { break; } }
                    _ => {}
                }
                i += 1;
            }
            arg = Some(&pattern[arg_start..i]);
            if i < len { i += 1; }
        }

        let mut value = match specifier {
            "d" | "date" => format_timestamp(&event.timestamp, arg.unwrap_or("")),

            "t" | "thread" | "tn" | "threadName" =>
                event.thread_name.clone().unwrap_or_else(|| "main".to_string()),

            "p" | "level" | "le" => level_name(event.level).to_string(),

            "c" | "logger" | "lo" =>
                format_logger_name(event.logger_name.as_deref().unwrap_or("root"), arg),

            "m" | "msg" | "message" => event.message.clone(),

            "n" => "\n".to_string(),

            // %X or %X{key} — MDC flat map
            "X" | "mdc" | "MDC" =>
                format_mdc(event.mdc.as_deref(), arg),

            // %x or %x{depth} — NDC stack
            "x" | "ndc" | "NDC" =>
                format_ndc(event.ndc.as_deref(), arg),

            // %ex — full exception stack trace (empty string when no exception)
            "ex" | "exception" | "throwable" | "xEx" | "xThrowable" | "rEx" =>
                event.exception.clone().unwrap_or_default(),

            "r" | "relative" => "0".to_string(),

            "highlight" => {
                let inner = if let Some(pat) = arg { apply_pattern(pat, event) }
                            else { level_name(event.level).to_string() };
                let color = match event.level {
                    1 => "\x1b[37m",   // TRACE  white
                    2 => "\x1b[34m",   // DEBUG  blue
                    3 => "\x1b[32m",   // INFO   green
                    4 => "\x1b[33m",   // WARN   yellow
                    _ => "\x1b[31m",   // ERROR  red
                };
                format!("{}{}\x1b[0m", color, inner)
            },

            _ => {
                let a = arg.map(|a| format!("{{{}}}", a)).unwrap_or_default();
                format!("%{}{}", specifier, a)
            }
        };

        // Truncate from left to max_width
        if let Some(max) = max_width {
            if value.len() > max {
                let start = value.len() - max;
                value = value[start..].to_string();
            }
        }

        // Pad to min_width
        if min_width > 0 && value.len() < min_width {
            value = if left_justify {
                format!("{:<w$}", value, w = min_width)
            } else {
                format!("{:>w$}", value, w = min_width)
            };
        }

        result.push_str(&value);
    }
    result
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"'  => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c    => out.push(c),
        }
    }
    out
}

fn format_json(event: &LogEvent) -> String {
    let ts     = event.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let level  = level_name(event.level);
    let logger = event.logger_name.as_deref().unwrap_or("root");
    let thread = event.thread_name.as_deref().unwrap_or("main");

    let mut json = format!(
        "{{\"timestamp\":\"{ts}\",\"level\":\"{level}\",\
          \"logger\":\"{logger}\",\"thread\":\"{thread}\",\
          \"message\":\"{msg}\"",
        ts = ts, level = level,
        logger = json_escape(logger),
        thread = json_escape(thread),
        msg = json_escape(&event.message),
    );

    if let Some(mdc) = &event.mdc {
        if !mdc.is_empty() {
            json.push_str(&format!(",\"mdc\":\"{}\"", json_escape(mdc)));
        }
    }
    if let Some(ndc) = &event.ndc {
        if !ndc.is_empty() {
            json.push_str(&format!(",\"ndc\":\"{}\"", json_escape(ndc)));
        }
    }
    if let Some(ex) = &event.exception {
        if !ex.is_empty() {
            json.push_str(&format!(",\"exception\":\"{}\"", json_escape(ex)));
        }
    }
    json.push_str("}\n");
    json
}

// ── Per-logger level hierarchy ────────────────────────────────────────────────

/// Walk the dotted logger name from most-specific to least-specific.
/// Returns the first matching PerLoggerConfig's level, or root_level if none match.
fn effective_logger_level(logger_name: &str, root_level: i32, loggers: &[PerLoggerConfig]) -> i32 {
    if loggers.is_empty() || logger_name.is_empty() {
        return root_level;
    }
    // Find the longest (most-specific) config name that is an exact match or
    // a prefix of logger_name followed by a dot.
    let mut best: Option<(&PerLoggerConfig, usize)> = None;
    for cfg in loggers {
        let is_match = logger_name == cfg.name
            || logger_name.starts_with(&format!("{}.", cfg.name));
        if is_match {
            let spec = cfg.name.len();
            match best {
                None => best = Some((cfg, spec)),
                Some((_, prev)) if spec > prev => best = Some((cfg, spec)),
                _ => {}
            }
        }
    }
    best.map(|(cfg, _)| cfg.min_level).unwrap_or(root_level)
}

// ── Event channel & background I/O thread ────────────────────────────────────

/// Build a file writer from the given config snapshot.
fn build_file_writer(
    file_name: &Option<String>,
    rolling_policy: &RollingPolicy,
    max_size: Option<u64>,
    max_files: usize,
) -> Option<Box<dyn Write + Send>> {
    let f_name = file_name.as_deref()?;
    let path = Path::new(f_name);
    let dir  = path.parent().unwrap_or(Path::new("."));
    if !dir.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Some(limit) = max_size {
        let cond = rolling_file::RollingConditionBasic::new().max_size(limit);
        match rolling_file::RollingFileAppender::new(f_name, cond, max_files) {
            Ok(a)  => Some(Box::new(a)),
            Err(e) => { eprintln!("Rlog4 Error: {}", e); None }
        }
    } else {
        let prefix = path.file_name().unwrap_or_default();
        match rolling_policy {
            RollingPolicy::Hourly =>
                Some(Box::new(tracing_appender::rolling::hourly(dir, prefix))),
            RollingPolicy::Daily =>
                Some(Box::new(tracing_appender::rolling::daily(dir, prefix))),
            RollingPolicy::Never =>
                match std::fs::OpenOptions::new().create(true).append(true).open(f_name) {
                    Ok(f)  => Some(Box::new(std::io::BufWriter::new(f))),
                    Err(e) => { eprintln!("Rlog4 Error: cannot open {}: {}", f_name, e); None }
                },
        }
    }
}

fn send_event(event: LogEvent) {
    if QUEUE_POLICY.load(Ordering::Relaxed) == 1 {
        let _ = LOG_SENDER.send(ChannelMessage::Log(event));
    } else {
        let _ = LOG_SENDER.try_send(ChannelMessage::Log(event));
    }
}

lazy_static! {
    static ref LOG_SENDER: Sender<ChannelMessage> = {
        let (sender, receiver) = bounded::<ChannelMessage>(1_000_000);

        thread::spawn(move || {
            // Take initial config snapshot
            let (mut file_name, mut rolling_policy, mut min_level, mut layout_type,
                 mut max_size, mut max_files, mut filters, mut pattern,
                 mut console_enabled, mut logger_configs) = {
                let cfg = CONFIG.lock().unwrap();
                (
                    cfg.file_name.clone(),
                    cfg.rolling_policy.clone(),
                    cfg.min_level,
                    cfg.layout_type.clone(),
                    cfg.max_size,
                    cfg.max_files,
                    cfg.filters.clone(),
                    cfg.pattern.clone(),
                    cfg.console_enabled,
                    cfg.loggers.clone(),
                )
            };

            let mut file_writer: Option<Box<dyn Write + Send>> =
                build_file_writer(&file_name, &rolling_policy, max_size, max_files);
            let mut use_console = console_enabled || file_writer.is_none();

            while let Ok(msg) = receiver.recv() {
                match msg {
                    ChannelMessage::Reconfigure => {
                        // Re-snapshot config and rebuild the file writer
                        let cfg = CONFIG.lock().unwrap();
                        file_name      = cfg.file_name.clone();
                        rolling_policy = cfg.rolling_policy.clone();
                        min_level      = cfg.min_level;
                        layout_type    = cfg.layout_type.clone();
                        max_size       = cfg.max_size;
                        max_files      = cfg.max_files;
                        filters        = cfg.filters.clone();
                        pattern        = cfg.pattern.clone();
                        console_enabled = cfg.console_enabled;
                        logger_configs = cfg.loggers.clone();
                        drop(cfg);

                        file_writer = build_file_writer(&file_name, &rolling_policy, max_size, max_files);
                        use_console = console_enabled || file_writer.is_none();
                        println!("Rlog4: configuration reloaded (root_level={} pattern='{}')", min_level, pattern);
                    }
                    ChannelMessage::Log(event) => {
                        // Per-logger level gate
                        let eff_level = effective_logger_level(
                            event.logger_name.as_deref().unwrap_or(""),
                            min_level,
                            &logger_configs,
                        );
                        if event.level < eff_level { continue; }

                        // Custom filters
                        let mut denied = false;
                        for filter in &filters {
                            match filter {
                                LogFilter::Threshold { min_level } => {
                                    if event.level < *min_level { denied = true; break; }
                                }
                                LogFilter::Regex { pattern, deny_on_match } => {
                                    let m = pattern.is_match(&event.message);
                                    if (m && *deny_on_match) || (!m && !*deny_on_match) {
                                        denied = true; break;
                                    }
                                }
                            }
                        }
                        if denied { continue; }

                        let formatted = match layout_type {
                            LayoutType::Json    => format_json(&event),
                            LayoutType::Pattern => apply_pattern(&pattern, &event),
                        };
                        let bytes = formatted.as_bytes();

                        if use_console {
                            let _ = std::io::stdout().write_all(bytes);
                        }
                        if let Some(ref mut fw) = file_writer {
                            let _ = fw.write_all(bytes);
                        }
                    }
                }
            }
        });

        sender
    };
}

// ── C API ─────────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rlog_configure(xml_ptr: *const c_char) -> i32 {
    if xml_ptr.is_null() { return -1; }
    let c_str = unsafe { CStr::from_ptr(xml_ptr) };
    let xml_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    println!("Rust received XML length: {}", xml_str.len());
    let mut reader = Reader::from_str(xml_str);
    reader.trim_text(true);

    let mut config = CONFIG.lock().unwrap();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let name_str = String::from_utf8_lossy(e.name().as_ref()).to_string();

                if name_str.eq_ignore_ascii_case("Console") {
                    println!("Rust detected Console appender");
                    config.console_enabled = true;

                } else if name_str.eq_ignore_ascii_case("PatternLayout") {
                    config.layout_type = LayoutType::Pattern;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"pattern" {
                            if let Ok(pat) = String::from_utf8(attr.value.into_owned()) {
                                println!("Rust extracted pattern: {}", pat);
                                config.pattern = pat;
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("JsonLayout") {
                    println!("Rust detected JsonLayout");
                    config.layout_type = LayoutType::Json;

                } else if name_str.eq_ignore_ascii_case("File") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"fileName" {
                            if let Ok(f) = String::from_utf8(attr.value.into_owned()) {
                                println!("Rust extracted file name: {}", f);
                                config.file_name = Some(f);
                                config.rolling_policy = RollingPolicy::Never;
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("RollingFile") {
                    let mut fname = None;
                    let mut policy = RollingPolicy::Never;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"fileName" {
                            if let Ok(f) = String::from_utf8(attr.value.into_owned()) {
                                fname = Some(f);
                            }
                        } else if attr.key.as_ref() == b"filePattern" {
                            if let Ok(p) = String::from_utf8(attr.value.into_owned()) {
                                if p.contains("HH") { policy = RollingPolicy::Hourly; }
                                else if p.contains("%d{") { policy = RollingPolicy::Daily; }
                            }
                        }
                    }
                    if let Some(f) = fname {
                        println!("Rust extracted rolling file name: {}", f);
                        config.file_name   = Some(f);
                        config.rolling_policy = policy;
                    }

                } else if name_str.eq_ignore_ascii_case("SizeBasedTriggeringPolicy") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"size" {
                            if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                let up = s.to_uppercase();
                                let bytes: u64 = if up.ends_with("MB") {
                                    up.trim_end_matches("MB").trim().parse::<u64>().unwrap_or(0) * 1024 * 1024
                                } else if up.ends_with("KB") {
                                    up.trim_end_matches("KB").trim().parse::<u64>().unwrap_or(0) * 1024
                                } else {
                                    up.parse::<u64>().unwrap_or(0)
                                };
                                println!("Rust configured SizeBasedTriggeringPolicy: {} bytes", bytes);
                                config.max_size = Some(bytes);
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("DefaultRolloverStrategy") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"max" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                if let Ok(n) = v.parse::<usize>() {
                                    println!("Rust configured DefaultRolloverStrategy max: {}", n);
                                    config.max_files = n;
                                }
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("Root") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"level" {
                            if let Ok(l) = String::from_utf8(attr.value.into_owned()) {
                                config.min_level = match l.to_uppercase().as_str() {
                                    "TRACE" => 1,
                                    "DEBUG" => 2,
                                    "INFO"  => 3,
                                    "WARN"  => 4,
                                    "ERROR" | "FATAL" => 5,
                                    _ => 3,
                                };
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("AsyncQueueFullPolicy") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" {
                            if let Ok(t) = String::from_utf8(attr.value.into_owned()) {
                                if t.eq_ignore_ascii_case("Block") {
                                    println!("Rust configured to BLOCK on full queue");
                                    QUEUE_POLICY.store(1, Ordering::SeqCst);
                                } else {
                                    QUEUE_POLICY.store(0, Ordering::SeqCst);
                                }
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("ThresholdFilter") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"level" {
                            if let Ok(l) = String::from_utf8(attr.value.into_owned()) {
                                let lvl = match l.to_uppercase().as_str() {
                                    "TRACE" => 1, "DEBUG" => 2, "INFO" => 3,
                                    "WARN"  => 4, "ERROR" | "FATAL" => 5, _ => 3,
                                };
                                println!("Rust configured ThresholdFilter min_level={}", lvl);
                                config.filters.push(LogFilter::Threshold { min_level: lvl });
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("RegexFilter") {
                    let mut pat_str: Option<String> = None;
                    let mut deny_on_match = true;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"regex" {
                            if let Ok(r) = String::from_utf8(attr.value.into_owned()) {
                                pat_str = Some(r);
                            }
                        } else if attr.key.as_ref() == b"onMatch" {
                            if let Ok(a) = String::from_utf8(attr.value.into_owned()) {
                                deny_on_match = a.eq_ignore_ascii_case("DENY");
                            }
                        }
                    }
                    if let Some(ps) = pat_str {
                        if let Ok(re) = regex::Regex::new(&ps) {
                            println!("Rust configured RegexFilter: pattern='{}' deny={}", ps, deny_on_match);
                            config.filters.push(LogFilter::Regex { pattern: re, deny_on_match });
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("Logger") {
                    // Per-logger level: <Logger name="com.example.db" level="WARN" additivity="true"/>
                    let mut logger_name = String::new();
                    let mut logger_level: Option<i32> = None;
                    let mut additivity = true;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"name" {
                            if let Ok(n) = String::from_utf8(attr.value.into_owned()) {
                                logger_name = n;
                            }
                        } else if attr.key.as_ref() == b"level" {
                            if let Ok(l) = String::from_utf8(attr.value.into_owned()) {
                                logger_level = Some(match l.to_uppercase().as_str() {
                                    "TRACE" => 1, "DEBUG" => 2, "INFO" => 3,
                                    "WARN"  => 4, "ERROR" | "FATAL" => 5, _ => 3,
                                });
                            }
                        } else if attr.key.as_ref() == b"additivity" {
                            if let Ok(a) = String::from_utf8(attr.value.into_owned()) {
                                additivity = !a.eq_ignore_ascii_case("false");
                            }
                        }
                    }
                    if !logger_name.is_empty() {
                        let lvl = logger_level.unwrap_or(config.min_level);
                        println!("Rust configured Logger '{}' level={} additivity={}", logger_name, lvl, additivity);
                        config.loggers.push(PerLoggerConfig {
                            name: logger_name,
                            min_level: lvl,
                            additivity,
                        });
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    // If the background thread is already running, tell it to reload config
    if INITIALIZED.load(Ordering::Relaxed) {
        let _ = LOG_SENDER.try_send(ChannelMessage::Reconfigure);
    }
    0
}

#[no_mangle]
pub extern "C" fn rlog_init() -> i32 {
    // Force lazy initialisation of the background I/O thread
    let _ = LOG_SENDER.len();
    INITIALIZED.store(true, Ordering::Relaxed);
    0
}

/// Runtime-only reconfigure entry point (alias for rlog_configure).
/// Called by Configurator.reconfigure() and the file-watcher path.
#[no_mangle]
pub extern "C" fn rlog_reconfigure(xml_ptr: *const c_char) -> i32 {
    rlog_configure(xml_ptr)
}

#[no_mangle]
pub extern "C" fn rlog_log(level: i32, msg_ptr: *const c_char) {
    rlog_log_with_context(level, msg_ptr, std::ptr::null());
}

#[no_mangle]
pub extern "C" fn rlog_log_with_context(level: i32, msg_ptr: *const c_char, ctx_ptr: *const c_char) {
    if msg_ptr.is_null() { return; }
    let c_str = unsafe { CStr::from_ptr(msg_ptr) };
    if let Ok(msg) = c_str.to_str() {
        let mdc = if ctx_ptr.is_null() { None } else {
            unsafe { CStr::from_ptr(ctx_ptr) }.to_str().ok().map(str::to_owned)
        };
        send_event(LogEvent {
            level,
            message: msg.to_owned(),
            mdc,
            ndc: None,
            logger_name: None,
            thread_name: None,
            exception: None,
            timestamp: Utc::now(),
        });
    }
}

#[no_mangle]
pub extern "C" fn rlog_flush() {
    while !LOG_SENDER.is_empty() {
        thread::sleep(std::time::Duration::from_millis(5));
    }
    // Give the background I/O thread a moment to finish any in-flight write
    thread::sleep(std::time::Duration::from_millis(50));
}

// ── JNI wrappers (Java 8+) ────────────────────────────────────────────────────

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jint;

fn jstring_to_opt(env: &mut JNIEnv, js: &JString) -> Option<String> {
    env.get_string(js).ok().map(|s| String::from(s))
        .filter(|s| !s.is_empty())
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1init(
    _env: JNIEnv, _class: JClass,
) -> jint {
    rlog_init()
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1configure(
    mut env: JNIEnv, _class: JClass, xml_content: JString,
) -> jint {
    if let Ok(s) = env.get_string(&xml_content) {
        rlog_configure(s.as_ptr())
    } else { -1 }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log(
    mut env: JNIEnv, _class: JClass, level: jint, message: JString,
) {
    if let Ok(msg) = env.get_string(&message) {
        rlog_log(level, msg.as_ptr());
    }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1with_1context(
    mut env: JNIEnv, _class: JClass, level: jint, message: JString, context: JString,
) {
    if let Ok(msg) = env.get_string(&message) {
        if let Ok(ctx) = env.get_string(&context) {
            rlog_log_with_context(level, msg.as_ptr(), ctx.as_ptr());
        } else {
            rlog_log(level, msg.as_ptr());
        }
    }
}

/// Full-featured log call: carries logger name, thread name, MDC, NDC, and
/// exception stack trace across JNI so all PatternLayout specifiers work.
#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1full(
    mut env: JNIEnv, _class: JClass,
    level: jint,
    message: JString,
    mdc: JString,
    ndc: JString,
    logger_name: JString,
    thread_name: JString,
    exception: JString,
) {
    let msg = match env.get_string(&message) {
        Ok(s) => String::from(s),
        Err(_) => return,
    };
    let mdc         = jstring_to_opt(&mut env, &mdc);
    let ndc         = jstring_to_opt(&mut env, &ndc);
    let logger_name = jstring_to_opt(&mut env, &logger_name);
    let thread_name = jstring_to_opt(&mut env, &thread_name);
    let exception   = jstring_to_opt(&mut env, &exception);

    send_event(LogEvent {
        level,
        message: msg,
        mdc,
        ndc,
        logger_name,
        thread_name,
        exception,
        timestamp: Utc::now(),
    });
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1reconfigure(
    mut env: JNIEnv, _class: JClass, xml_content: JString,
) -> jint {
    if let Ok(s) = env.get_string(&xml_content) {
        rlog_reconfigure(s.as_ptr())
    } else { -1 }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1flush(
    _env: JNIEnv, _class: JClass,
) {
    rlog_flush();
}
