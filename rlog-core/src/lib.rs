use std::ffi::CStr;
use std::os::raw::c_char;
use crossbeam_channel::{bounded, Sender};
use lazy_static::lazy_static;
use std::thread;
use std::sync::{Arc, Mutex};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use chrono::{DateTime, Utc};
use std::io::Write;

// ── Globals ───────────────────────────────────────────────────────────────────

// Queue backpressure policy: 0 = Discard, 1 = Block
static QUEUE_POLICY: AtomicU8 = AtomicU8::new(0);

// Set to true once rlog_init() has been called (background thread is running)
static INITIALIZED: AtomicBool = AtomicBool::new(false);

// ── Enums & structs ───────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum RollingPolicy { Never, Hourly, Daily }

#[derive(Clone, PartialEq)]
enum LayoutType { Pattern, Json, Html, Xml, Csv }

struct BurstState {
    count: u32,
    window_start: std::time::Instant,
}

#[derive(Clone)]
enum LogFilter {
    Threshold { min_level: i32 },
    Regex     { pattern: regex::Regex, deny_on_match: bool },
    /// Rate-limit: allow at most max_count events per interval_millis window.
    Burst     { max_count: u32, interval_millis: u64, deny_when_full: bool,
                state: Arc<Mutex<BurstState>> },
    /// Accept/deny based on the event's marker name.
    Marker    { marker_name: String, deny_on_match: bool },
    /// Look up a level name from the MDC and use it as the per-event threshold.
    DynamicThreshold { mdc_key: String, default_level: i32 },
}

// LogFilter must be Clone (background thread takes a snapshot).
// Arc<Mutex<>> inside Burst is cheaply cloneable.
impl Clone for BurstState {
    fn clone(&self) -> Self {
        BurstState { count: self.count, window_start: self.window_start }
    }
}

/// Per-logger level override (mirrors <Logger name="..." level="..." additivity="..."/>)
#[derive(Clone)]
struct PerLoggerConfig {
    name:       String,
    min_level:  i32,
    additivity: bool,
}

struct RlogConfig {
    // File / rolling appender
    file_name:       Option<String>,
    rolling_policy:  RollingPolicy,
    max_size:        Option<u64>,
    max_files:       usize,
    // Socket appender (TCP)
    socket_host:     Option<String>,
    socket_port:     u16,
    // Syslog appender (UDP, RFC 3164)
    syslog_host:     Option<String>,
    syslog_port:     u16,
    syslog_facility: u8,
    // HTTP appender (POST)
    http_url:        Option<String>,
    http_method:     String,
    // Layout
    layout_type:     LayoutType,
    pattern:         String,
    csv_delimiter:   char,
    // Level / filters
    min_level:       i32,
    filters:         Vec<LogFilter>,
    console_enabled: bool,
    loggers:         Vec<PerLoggerConfig>,
    // Caller location capture (requires Java-side stack walk)
    include_location: bool,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name:        None,
        rolling_policy:   RollingPolicy::Never,
        max_size:         None,
        max_files:        7,
        socket_host:      None,
        socket_port:      4560,
        syslog_host:      None,
        syslog_port:      514,
        syslog_facility:  1,  // USER
        http_url:         None,
        http_method:      "POST".to_string(),
        layout_type:      LayoutType::Pattern,
        pattern:          "%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n%ex".to_string(),
        csv_delimiter:    ',',
        min_level:        1,
        filters:          Vec::new(),
        console_enabled:  false,
        loggers:          Vec::new(),
        include_location: false,
    });
}

struct LogEvent {
    level:         i32,
    message:       String,
    mdc:           Option<String>,   // "{k=v, k2=v2}"
    ndc:           Option<String>,   // "outer inner"
    logger_name:   Option<String>,
    thread_name:   Option<String>,
    exception:     Option<String>,   // full printStackTrace output
    timestamp:     DateTime<Utc>,
    // Feature 9: Marker
    marker:        Option<String>,
    // Feature 8: Caller location (populated by Java via StackWalker)
    caller_class:  Option<String>,
    caller_method: Option<String>,
    caller_file:   Option<String>,
    caller_line:   Option<i32>,
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

/// Java SimpleDateFormat → chrono formatted string.
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
        if chars[i] == '\'' {
            i += 1;
            while i < n && chars[i] != '\'' { result.push(chars[i]); i += 1; }
            if i < n { i += 1; }
            continue;
        }
        let ch = chars[i];
        if !ch.is_ascii_alphabetic() { result.push(ch); i += 1; continue; }
        let mut count = 0usize;
        while i + count < n && chars[i + count] == ch { count += 1; }
        i += count;
        let seg = match ch {
            'y' => ts.format("%Y").to_string(),
            'M' => if count >= 3 { ts.format("%b").to_string() } else { ts.format("%m").to_string() },
            'd' => ts.format("%d").to_string(),
            'H' => ts.format("%H").to_string(),
            'h' => ts.format("%I").to_string(),
            'm' => ts.format("%M").to_string(),
            's' => ts.format("%S").to_string(),
            'S' => { let s = format!("{:03}", millis); s[..count.min(3)].to_string() },
            'a' => ts.format("%p").to_string(),
            'E' => if count >= 4 { ts.format("%A").to_string() } else { ts.format("%a").to_string() },
            'z' | 'Z' | 'X' => ts.format("%z").to_string(),
            _ => std::iter::repeat(ch).take(count).collect(),
        };
        result.push_str(&seg);
    }
    result
}

/// Abbreviate a fully-qualified class name to at most `max_len` chars (Log4j2 behaviour).
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
            if idx == parts.len() - 1 { abbrev.push_str(part); }
            else { abbrev.push(part.chars().next().unwrap_or('?')); }
        }
        if abbrev.len() <= max_len { abbrev } else { class.to_string() }
    } else {
        name.to_string()
    }
}

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

fn format_ndc(ndc: Option<&str>, depth: Option<&str>) -> String {
    match ndc {
        None | Some("") => String::new(),
        Some(s) => {
            if let Some(d) = depth.and_then(|d| d.parse::<usize>().ok()) {
                let parts: Vec<&str> = s.split(' ').collect();
                if d >= parts.len() { return s.to_string(); }
                parts[parts.len() - d..].join(" ")
            } else { s.to_string() }
        }
    }
}

/// Apply a Log4j2-style PatternLayout pattern to a log event.
fn apply_pattern(pattern: &str, event: &LogEvent) -> String {
    let mut result = String::with_capacity(256);
    let bytes = pattern.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] != b'%' { result.push(bytes[i] as char); i += 1; continue; }
        i += 1;
        if i >= len { break; }
        if bytes[i] == b'%' { result.push('%'); i += 1; continue; }

        let left_justify = if i < len && bytes[i] == b'-' { i += 1; true } else { false };

        let mut min_width = 0usize;
        while i < len && bytes[i].is_ascii_digit() {
            min_width = min_width * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }
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

        let spec_start = i;
        while i < len && bytes[i].is_ascii_alphabetic() { i += 1; }
        let specifier = &pattern[spec_start..i];

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
            "d" | "date"   => format_timestamp(&event.timestamp, arg.unwrap_or("")),
            "t" | "thread" | "tn" | "threadName" =>
                event.thread_name.clone().unwrap_or_else(|| "main".to_string()),
            "p" | "level" | "le" => level_name(event.level).to_string(),
            "c" | "logger" | "lo" =>
                format_logger_name(event.logger_name.as_deref().unwrap_or("root"), arg),
            "m" | "msg" | "message" => event.message.clone(),
            "n"  => "\n".to_string(),
            "X" | "mdc" | "MDC" => format_mdc(event.mdc.as_deref(), arg),
            "x" | "ndc" | "NDC" => format_ndc(event.ndc.as_deref(), arg),
            "ex" | "exception" | "throwable" | "xEx" | "xThrowable" | "rEx" =>
                event.exception.clone().unwrap_or_default(),
            "r" | "relative" => "0".to_string(),
            // Feature 9: Marker
            "marker" | "markerSimpleName" =>
                event.marker.clone().unwrap_or_default(),
            // Feature 8: Caller location
            "C" | "class"  => event.caller_class.clone().unwrap_or_else(|| "?".to_string()),
            "M" | "method" => event.caller_method.clone().unwrap_or_else(|| "?".to_string()),
            "F" | "file"   => event.caller_file.clone().unwrap_or_else(|| "?".to_string()),
            "L" | "line"   => event.caller_line.map(|l| l.to_string()).unwrap_or_else(|| "?".to_string()),
            "highlight" => {
                let inner = if let Some(pat) = arg { apply_pattern(pat, event) }
                            else { level_name(event.level).to_string() };
                let color = match event.level {
                    1 => "\x1b[37m", 2 => "\x1b[34m", 3 => "\x1b[32m",
                    4 => "\x1b[33m", _ => "\x1b[31m",
                };
                format!("{}{}\x1b[0m", color, inner)
            },
            _ => { let a = arg.map(|a| format!("{{{}}}", a)).unwrap_or_default();
                   format!("%{}{}", specifier, a) }
        };

        if let Some(max) = max_width {
            if value.len() > max { let s = value.len() - max; value = value[s..].to_string(); }
        }
        if min_width > 0 && value.len() < min_width {
            value = if left_justify { format!("{:<w$}", value, w = min_width) }
                    else            { format!("{:>w$}", value, w = min_width) };
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
            c => out.push(c),
        }
    }
    out
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&'  => out.push_str("&amp;"),
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c    => out.push(c),
        }
    }
    out
}

fn xml_escape(s: &str) -> String { html_escape(s) }

fn csv_escape(s: &str, delimiter: char) -> String {
    if s.contains(delimiter) || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn format_json(event: &LogEvent) -> String {
    let ts     = event.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let level  = level_name(event.level);
    let logger = event.logger_name.as_deref().unwrap_or("root");
    let thread = event.thread_name.as_deref().unwrap_or("main");
    let mut json = format!(
        "{{\"timestamp\":\"{ts}\",\"level\":\"{level}\",\"logger\":\"{logger}\",\
          \"thread\":\"{thread}\",\"message\":\"{msg}\"",
        ts = ts, level = level,
        logger = json_escape(logger), thread = json_escape(thread),
        msg = json_escape(&event.message),
    );
    if let Some(m) = &event.marker {
        if !m.is_empty() { json.push_str(&format!(",\"marker\":\"{}\"", json_escape(m))); }
    }
    if let Some(mdc) = &event.mdc {
        if !mdc.is_empty() { json.push_str(&format!(",\"mdc\":\"{}\"", json_escape(mdc))); }
    }
    if let Some(ndc) = &event.ndc {
        if !ndc.is_empty() { json.push_str(&format!(",\"ndc\":\"{}\"", json_escape(ndc))); }
    }
    if let Some(ex) = &event.exception {
        if !ex.is_empty() { json.push_str(&format!(",\"exception\":\"{}\"", json_escape(ex))); }
    }
    if let Some(cls) = &event.caller_class {
        json.push_str(&format!(",\"caller_class\":\"{}\"", json_escape(cls)));
        if let Some(m) = &event.caller_method { json.push_str(&format!(",\"caller_method\":\"{}\"", json_escape(m))); }
        if let Some(f) = &event.caller_file   { json.push_str(&format!(",\"caller_file\":\"{}\"", json_escape(f))); }
        if let Some(l) = event.caller_line    { json.push_str(&format!(",\"caller_line\":{}", l)); }
    }
    json.push_str("}\n");
    json
}

/// Feature 7: HTML layout — one <tr> per event (no surrounding <table>; caller wraps it).
fn format_html(event: &LogEvent) -> String {
    let ts     = event.timestamp.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
    let level  = level_name(event.level);
    let logger = html_escape(event.logger_name.as_deref().unwrap_or("root"));
    let thread = html_escape(event.thread_name.as_deref().unwrap_or("main"));
    let msg    = html_escape(&event.message);
    let color  = match event.level {
        1 => "#aaa", 2 => "#36f", 3 => "#090",
        4 => "#f80", _ => "#e00",
    };
    let mut row = format!(
        "<tr><td>{ts}</td><td><b style=\"color:{color}\">{level}</b></td>\
          <td>{thread}</td><td>{logger}</td><td>{msg}</td></tr>\n",
    );
    if let Some(ex) = &event.exception {
        if !ex.is_empty() {
            row.push_str(&format!("<tr><td colspan=\"5\"><pre>{}</pre></td></tr>\n",
                html_escape(ex)));
        }
    }
    row
}

/// Feature 7: XML layout (log4j event format).
fn format_xml_event(event: &LogEvent) -> String {
    let ts_ms  = event.timestamp.timestamp_millis();
    let level  = level_name(event.level);
    let logger = xml_escape(event.logger_name.as_deref().unwrap_or("root"));
    let thread = xml_escape(event.thread_name.as_deref().unwrap_or("main"));
    let msg    = xml_escape(&event.message);
    let mut s = format!(
        "<log4j:event logger=\"{logger}\" timestamp=\"{ts_ms}\" level=\"{level}\" thread=\"{thread}\">\n\
           <log4j:message><![CDATA[{msg}]]></log4j:message>\n",
    );
    if let Some(ndc) = &event.ndc {
        if !ndc.is_empty() {
            s.push_str(&format!("  <log4j:NDC><![CDATA[{}]]></log4j:NDC>\n", ndc));
        }
    }
    if let Some(ex) = &event.exception {
        if !ex.is_empty() {
            s.push_str("  <log4j:throwable><![CDATA[");
            s.push_str(ex);
            s.push_str("]]></log4j:throwable>\n");
        }
    }
    if let Some(mdc) = &event.mdc {
        if !mdc.is_empty() {
            s.push_str("  <log4j:properties>\n");
            let inner = mdc.trim_matches(|c| c == '{' || c == '}');
            for pair in inner.split(", ") {
                let mut kv = pair.splitn(2, '=');
                if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                    s.push_str(&format!("    <log4j:data name=\"{}\" value=\"{}\"/>\n",
                        xml_escape(k.trim()), xml_escape(v.trim())));
                }
            }
            s.push_str("  </log4j:properties>\n");
        }
    }
    s.push_str("</log4j:event>\n");
    s
}

/// Feature 7: CSV layout.
fn format_csv(event: &LogEvent, delimiter: char) -> String {
    let d = delimiter;
    let ts     = event.timestamp.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let level  = level_name(event.level);
    let logger = event.logger_name.as_deref().unwrap_or("root");
    let thread = event.thread_name.as_deref().unwrap_or("main");
    format!("{}{d}{}{d}{}{d}{}{d}{}\n",
        csv_escape(&ts, d), csv_escape(level, d),
        csv_escape(thread, d), csv_escape(logger, d),
        csv_escape(&event.message, d))
}

// ── Per-logger level hierarchy ────────────────────────────────────────────────

fn effective_logger_level(logger_name: &str, root_level: i32, loggers: &[PerLoggerConfig]) -> i32 {
    if loggers.is_empty() || logger_name.is_empty() { return root_level; }
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

// ── HTTP appender writer ──────────────────────────────────────────────────────

struct HttpWriter {
    url:    String,
    method: String,
    buf:    Vec<u8>,
}

impl HttpWriter {
    fn new(url: String, method: String) -> Self {
        Self { url, method, buf: Vec::new() }
    }

    fn flush_http(&mut self) {
        if self.buf.is_empty() { return; }
        let body = std::mem::take(&mut self.buf);
        let body_str = String::from_utf8_lossy(&body).to_string();
        let result = if self.method.eq_ignore_ascii_case("POST") {
            ureq::post(&self.url)
                .set("Content-Type", "text/plain; charset=utf-8")
                .send_string(&body_str)
        } else {
            ureq::put(&self.url)
                .set("Content-Type", "text/plain; charset=utf-8")
                .send_string(&body_str)
        };
        if let Err(e) = result {
            eprintln!("Rlog4 HttpAppender error: {}", e);
        }
    }
}

impl Write for HttpWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        // Auto-flush on newline boundaries (keep each log line as one POST)
        if self.buf.ends_with(b"\n") {
            self.flush_http();
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_http();
        Ok(())
    }
}

// ── Syslog UDP writer ─────────────────────────────────────────────────────────

struct SyslogWriter {
    socket:   std::net::UdpSocket,
    addr:     String,
    facility: u8,
}

impl SyslogWriter {
    fn new(host: &str, port: u16, facility: u8) -> Option<Self> {
        let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        Some(Self { socket, addr: format!("{}:{}", host, port), facility })
    }

    fn severity(level: i32) -> u8 {
        match level { 1 => 7, 2 => 7, 3 => 6, 4 => 4, _ => 3 }  // debug/debug/info/warning/error
    }

    fn send(&self, level: i32, msg: &str) {
        let pri = (self.facility << 3) | Self::severity(level);
        let packet = format!("<{}>{}", pri, msg);
        let _ = self.socket.send_to(packet.as_bytes(), &self.addr);
    }
}

// ── Appender snapshot ─────────────────────────────────────────────────────────

/// Holds everything the background thread needs for one reconfiguration snapshot.
struct ThreadSnapshot {
    file_name:        Option<String>,
    rolling_policy:   RollingPolicy,
    max_size:         Option<u64>,
    max_files:        usize,
    socket_host:      Option<String>,
    socket_port:      u16,
    syslog_host:      Option<String>,
    syslog_port:      u16,
    syslog_facility:  u8,
    http_url:         Option<String>,
    http_method:      String,
    layout_type:      LayoutType,
    pattern:          String,
    csv_delimiter:    char,
    min_level:        i32,
    filters:          Vec<LogFilter>,
    console_enabled:  bool,
    logger_configs:   Vec<PerLoggerConfig>,
}

fn snapshot_config() -> ThreadSnapshot {
    let cfg = CONFIG.lock().unwrap();
    ThreadSnapshot {
        file_name:       cfg.file_name.clone(),
        rolling_policy:  cfg.rolling_policy.clone(),
        max_size:        cfg.max_size,
        max_files:       cfg.max_files,
        socket_host:     cfg.socket_host.clone(),
        socket_port:     cfg.socket_port,
        syslog_host:     cfg.syslog_host.clone(),
        syslog_port:     cfg.syslog_port,
        syslog_facility: cfg.syslog_facility,
        http_url:        cfg.http_url.clone(),
        http_method:     cfg.http_method.clone(),
        layout_type:     cfg.layout_type.clone(),
        pattern:         cfg.pattern.clone(),
        csv_delimiter:   cfg.csv_delimiter,
        min_level:       cfg.min_level,
        filters:         cfg.filters.clone(),
        console_enabled: cfg.console_enabled,
        logger_configs:  cfg.loggers.clone(),
    }
}

// ── File-writer factory ───────────────────────────────────────────────────────

fn build_file_writer(
    file_name: &Option<String>,
    rolling_policy: &RollingPolicy,
    max_size: Option<u64>,
    max_files: usize,
) -> Option<Box<dyn Write + Send>> {
    let f_name = file_name.as_deref()?;
    let path = Path::new(f_name);
    let dir  = path.parent().unwrap_or(Path::new("."));
    if !dir.as_os_str().is_empty() { let _ = std::fs::create_dir_all(dir); }
    if let Some(limit) = max_size {
        let cond = rolling_file::RollingConditionBasic::new().max_size(limit);
        match rolling_file::RollingFileAppender::new(f_name, cond, max_files) {
            Ok(a)  => Some(Box::new(a)),
            Err(e) => { eprintln!("Rlog4 Error: {}", e); None }
        }
    } else {
        let prefix = path.file_name().unwrap_or_default();
        match rolling_policy {
            RollingPolicy::Hourly => Some(Box::new(tracing_appender::rolling::hourly(dir, prefix))),
            RollingPolicy::Daily  => Some(Box::new(tracing_appender::rolling::daily(dir, prefix))),
            RollingPolicy::Never  =>
                match std::fs::OpenOptions::new().create(true).append(true).open(f_name) {
                    Ok(f)  => Some(Box::new(std::io::BufWriter::new(f))),
                    Err(e) => { eprintln!("Rlog4 Error: cannot open {}: {}", f_name, e); None }
                },
        }
    }
}

// ── Filter evaluation ─────────────────────────────────────────────────────────

fn apply_filters(event: &LogEvent, filters: &mut Vec<LogFilter>) -> bool {
    for filter in filters.iter_mut() {
        let denied = match filter {
            LogFilter::Threshold { min_level } => event.level < *min_level,

            LogFilter::Regex { pattern, deny_on_match } => {
                let m = pattern.is_match(&event.message);
                (m && *deny_on_match) || (!m && !*deny_on_match)
            }

            LogFilter::Marker { marker_name, deny_on_match } => {
                let matches = event.marker.as_deref()
                    .map(|m| m == marker_name.as_str()).unwrap_or(false);
                (matches && *deny_on_match) || (!matches && !*deny_on_match)
            }

            LogFilter::DynamicThreshold { mdc_key, default_level } => {
                let threshold = event.mdc.as_deref()
                    .and_then(|mdc| {
                        let inner = mdc.trim_matches(|c| c == '{' || c == '}');
                        inner.split(", ").find_map(|pair| {
                            let mut kv = pair.splitn(2, '=');
                            let k = kv.next()?.trim();
                            let v = kv.next()?.trim();
                            if k == mdc_key.as_str() { Some(parse_level_str(v)) } else { None }
                        })
                    })
                    .unwrap_or(*default_level);
                event.level < threshold
            }

            LogFilter::Burst { max_count, interval_millis, deny_when_full, state } => {
                let mut st = state.lock().unwrap();
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(st.window_start).as_millis() as u64;
                if elapsed >= *interval_millis {
                    st.count = 0;
                    st.window_start = now;
                }
                if st.count < *max_count {
                    st.count += 1;
                    false  // allow
                } else {
                    *deny_when_full  // deny if configured to do so
                }
            }
        };
        if denied { return true; }  // denied
    }
    false  // allowed
}

fn parse_level_str(s: &str) -> i32 {
    match s.to_uppercase().as_str() {
        "TRACE" => 1, "DEBUG" => 2, "INFO" => 3, "WARN" => 4,
        "ERROR" | "FATAL" => 5, _ => 3,
    }
}

// ── Event channel & background I/O thread ────────────────────────────────────

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
            let mut snap = snapshot_config();

            let mut file_writer: Option<Box<dyn Write + Send>> =
                build_file_writer(&snap.file_name, &snap.rolling_policy, snap.max_size, snap.max_files);

            // Socket (TCP)
            let mut socket_writer: Option<Box<dyn Write + Send>> =
                snap.socket_host.as_deref().and_then(|h| {
                    std::net::TcpStream::connect(format!("{}:{}", h, snap.socket_port)).ok()
                        .map(|s| -> Box<dyn Write + Send> { Box::new(s) })
                });

            // Syslog (UDP)
            let mut syslog_writer: Option<SyslogWriter> =
                snap.syslog_host.as_deref().and_then(|h|
                    SyslogWriter::new(h, snap.syslog_port, snap.syslog_facility));

            // HTTP
            let mut http_writer: Option<HttpWriter> =
                snap.http_url.as_ref().map(|url|
                    HttpWriter::new(url.clone(), snap.http_method.clone()));

            let mut use_console = snap.console_enabled || file_writer.is_none();

            while let Ok(msg) = receiver.recv() {
                match msg {
                    ChannelMessage::Reconfigure => {
                        snap = snapshot_config();
                        file_writer   = build_file_writer(&snap.file_name, &snap.rolling_policy,
                                                          snap.max_size, snap.max_files);
                        socket_writer = snap.socket_host.as_deref().and_then(|h|
                            std::net::TcpStream::connect(format!("{}:{}", h, snap.socket_port)).ok()
                                .map(|s| -> Box<dyn Write + Send> { Box::new(s) }));
                        syslog_writer = snap.syslog_host.as_deref().and_then(|h|
                            SyslogWriter::new(h, snap.syslog_port, snap.syslog_facility));
                        http_writer   = snap.http_url.as_ref().map(|url|
                            HttpWriter::new(url.clone(), snap.http_method.clone()));
                        use_console   = snap.console_enabled || file_writer.is_none();
                        println!("Rlog4: configuration reloaded (root_level={} pattern='{}')",
                                 snap.min_level, snap.pattern);
                    }

                    ChannelMessage::Log(event) => {
                        // Per-logger level gate
                        let eff = effective_logger_level(
                            event.logger_name.as_deref().unwrap_or(""),
                            snap.min_level, &snap.logger_configs,
                        );
                        if event.level < eff { continue; }

                        // Feature 6: filters
                        if apply_filters(&event, &mut snap.filters) { continue; }

                        // Format
                        let formatted = match snap.layout_type {
                            LayoutType::Json    => format_json(&event),
                            LayoutType::Pattern => apply_pattern(&snap.pattern, &event),
                            LayoutType::Html    => format_html(&event),
                            LayoutType::Xml     => format_xml_event(&event),
                            LayoutType::Csv     => format_csv(&event, snap.csv_delimiter),
                        };
                        let bytes = formatted.as_bytes();

                        if use_console { let _ = std::io::stdout().write_all(bytes); }
                        if let Some(ref mut fw) = file_writer   { let _ = fw.write_all(bytes); }
                        if let Some(ref mut sw) = socket_writer { let _ = sw.write_all(bytes); }
                        if let Some(ref mut hw) = http_writer   { let _ = hw.write_all(bytes); }
                        if let Some(ref sw) = syslog_writer {
                            sw.send(event.level, formatted.trim_end_matches('\n'));
                        }
                    }
                }
            }
        });

        sender
    };
}

// ── XML config parser ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn rlog_configure(xml_ptr: *const c_char) -> i32 {
    if xml_ptr.is_null() { return -1; }
    let c_str = unsafe { CStr::from_ptr(xml_ptr) };
    let xml_str = match c_str.to_str() { Ok(s) => s, Err(_) => return -1 };

    println!("Rust received XML length: {}", xml_str.len());
    let mut reader = Reader::from_str(xml_str);
    reader.trim_text(true);

    let mut config = CONFIG.lock().unwrap();
    // Reset filters on every reconfigure to avoid duplicates
    config.filters.clear();
    config.loggers.clear();
    config.console_enabled = false;
    config.socket_host = None;
    config.syslog_host = None;
    config.http_url    = None;
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

                // Feature 7 ─────────────────────────────────────────────────
                } else if name_str.eq_ignore_ascii_case("HTMLLayout") {
                    println!("Rust detected HTMLLayout");
                    config.layout_type = LayoutType::Html;

                } else if name_str.eq_ignore_ascii_case("XMLLayout") {
                    println!("Rust detected XMLLayout");
                    config.layout_type = LayoutType::Xml;

                } else if name_str.eq_ignore_ascii_case("CSVPatternLayout")
                       || name_str.eq_ignore_ascii_case("CSVLayout") {
                    config.layout_type = LayoutType::Csv;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"delimiter" {
                            if let Ok(d) = String::from_utf8(attr.value.into_owned()) {
                                config.csv_delimiter = d.chars().next().unwrap_or(',');
                            }
                        }
                    }
                    println!("Rust detected CSVLayout delimiter='{}'", config.csv_delimiter);

                // File appenders ─────────────────────────────────────────────
                } else if name_str.eq_ignore_ascii_case("File") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"fileName" {
                            if let Ok(f) = String::from_utf8(attr.value.into_owned()) {
                                println!("Rust extracted file name: {}", f);
                                config.file_name      = Some(f);
                                config.rolling_policy = RollingPolicy::Never;
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("RollingFile") {
                    let mut fname = None;
                    let mut policy = RollingPolicy::Never;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"fileName" {
                            if let Ok(f) = String::from_utf8(attr.value.into_owned()) { fname = Some(f); }
                        } else if attr.key.as_ref() == b"filePattern" {
                            if let Ok(p) = String::from_utf8(attr.value.into_owned()) {
                                if p.contains("HH") { policy = RollingPolicy::Hourly; }
                                else if p.contains("%d{") { policy = RollingPolicy::Daily; }
                            }
                        }
                    }
                    if let Some(f) = fname {
                        println!("Rust extracted rolling file name: {}", f);
                        config.file_name      = Some(f);
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
                                } else { up.parse::<u64>().unwrap_or(0) };
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

                // Feature 5: additional appenders ────────────────────────────
                } else if name_str.eq_ignore_ascii_case("Socket") {
                    let mut host = None; let mut port = 4560u16; let mut _proto = "TCP";
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"host" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { host = Some(v); }
                        } else if attr.key.as_ref() == b"port" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                port = v.parse().unwrap_or(4560);
                            }
                        }
                    }
                    if let Some(h) = host {
                        println!("Rust detected SocketAppender {}:{}", h, port);
                        config.socket_host = Some(h);
                        config.socket_port = port;
                    }

                } else if name_str.eq_ignore_ascii_case("Syslog") {
                    let mut host = None; let mut port = 514u16; let mut facility = 1u8;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"host" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { host = Some(v); }
                        } else if attr.key.as_ref() == b"port" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                port = v.parse().unwrap_or(514);
                            }
                        } else if attr.key.as_ref() == b"facility" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                facility = match v.to_uppercase().as_str() {
                                    "KERN" => 0, "USER" => 1, "MAIL" => 2, "DAEMON" => 3,
                                    "AUTH" => 4, "SYSLOG" => 5, "LPR" => 6, "NEWS" => 7,
                                    "LOCAL0" => 16, "LOCAL1" => 17, "LOCAL2" => 18, "LOCAL3" => 19,
                                    "LOCAL4" => 20, "LOCAL5" => 21, "LOCAL6" => 22, "LOCAL7" => 23,
                                    _ => 1,
                                };
                            }
                        }
                    }
                    if let Some(h) = host {
                        println!("Rust detected SyslogAppender {}:{} facility={}", h, port, facility);
                        config.syslog_host     = Some(h);
                        config.syslog_port     = port;
                        config.syslog_facility = facility;
                    }

                } else if name_str.eq_ignore_ascii_case("Http") {
                    let mut url = None; let mut method = "POST".to_string();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"url" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { url = Some(v); }
                        } else if attr.key.as_ref() == b"method" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { method = v; }
                        }
                    }
                    if let Some(u) = url {
                        println!("Rust detected HttpAppender {} {}", method, u);
                        config.http_url    = Some(u);
                        config.http_method = method;
                    }

                // Root / Logger ───────────────────────────────────────────────
                } else if name_str.eq_ignore_ascii_case("Root") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"level" {
                            if let Ok(l) = String::from_utf8(attr.value.into_owned()) {
                                config.min_level = parse_level_str(&l);
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("Logger") {
                    let mut lname = String::new(); let mut llevel = None; let mut additivity = true;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"name" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { lname = v; }
                        } else if attr.key.as_ref() == b"level" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                llevel = Some(parse_level_str(&v));
                            }
                        } else if attr.key.as_ref() == b"additivity" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                additivity = !v.eq_ignore_ascii_case("false");
                            }
                        }
                    }
                    if !lname.is_empty() {
                        let lvl = llevel.unwrap_or(config.min_level);
                        println!("Rust configured Logger '{}' level={} additivity={}", lname, lvl, additivity);
                        config.loggers.push(PerLoggerConfig { name: lname, min_level: lvl, additivity });
                    }

                // Async queue policy ──────────────────────────────────────────
                } else if name_str.eq_ignore_ascii_case("AsyncQueueFullPolicy") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"type" {
                            if let Ok(t) = String::from_utf8(attr.value.into_owned()) {
                                if t.eq_ignore_ascii_case("Block") {
                                    QUEUE_POLICY.store(1, Ordering::SeqCst);
                                } else { QUEUE_POLICY.store(0, Ordering::SeqCst); }
                            }
                        }
                    }

                // Feature 6: filters ──────────────────────────────────────────
                } else if name_str.eq_ignore_ascii_case("ThresholdFilter") {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"level" {
                            if let Ok(l) = String::from_utf8(attr.value.into_owned()) {
                                let lvl = parse_level_str(&l);
                                println!("Rust configured ThresholdFilter min_level={}", lvl);
                                config.filters.push(LogFilter::Threshold { min_level: lvl });
                            }
                        }
                    }

                } else if name_str.eq_ignore_ascii_case("RegexFilter") {
                    let mut pat_str = None; let mut deny_on_match = true;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"regex" {
                            if let Ok(r) = String::from_utf8(attr.value.into_owned()) { pat_str = Some(r); }
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

                } else if name_str.eq_ignore_ascii_case("MarkerFilter") {
                    let mut marker_name = String::new(); let mut deny_on_match = false;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"marker" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { marker_name = v; }
                        } else if attr.key.as_ref() == b"onMatch" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                deny_on_match = v.eq_ignore_ascii_case("DENY");
                            }
                        }
                    }
                    if !marker_name.is_empty() {
                        println!("Rust configured MarkerFilter marker='{}' deny={}", marker_name, deny_on_match);
                        config.filters.push(LogFilter::Marker { marker_name, deny_on_match });
                    }

                } else if name_str.eq_ignore_ascii_case("BurstFilter") {
                    let mut max_count = 100u32; let mut interval_millis = 1000u64;
                    let mut deny_when_full = true;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"maxBurst" || attr.key.as_ref() == b"rate" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                max_count = v.parse().unwrap_or(100);
                            }
                        } else if attr.key.as_ref() == b"burstInterval"
                               || attr.key.as_ref() == b"intervalMillis" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                interval_millis = v.parse().unwrap_or(1000);
                            }
                        } else if attr.key.as_ref() == b"onMatch" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                deny_when_full = v.eq_ignore_ascii_case("DENY");
                            }
                        }
                    }
                    println!("Rust configured BurstFilter max={} interval={}ms deny={}", max_count, interval_millis, deny_when_full);
                    config.filters.push(LogFilter::Burst {
                        max_count, interval_millis, deny_when_full,
                        state: Arc::new(Mutex::new(BurstState {
                            count: 0, window_start: std::time::Instant::now(),
                        })),
                    });

                } else if name_str.eq_ignore_ascii_case("DynamicThresholdFilter") {
                    let mut mdc_key = String::new(); let mut default_level = 3i32;
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"key" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) { mdc_key = v; }
                        } else if attr.key.as_ref() == b"defaultThreshold" {
                            if let Ok(v) = String::from_utf8(attr.value.into_owned()) {
                                default_level = parse_level_str(&v);
                            }
                        }
                    }
                    if !mdc_key.is_empty() {
                        println!("Rust configured DynamicThresholdFilter key='{}' default={}", mdc_key, default_level);
                        config.filters.push(LogFilter::DynamicThreshold { mdc_key, default_level });
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    if INITIALIZED.load(Ordering::Relaxed) {
        let _ = LOG_SENDER.try_send(ChannelMessage::Reconfigure);
    }
    0
}

#[no_mangle]
pub extern "C" fn rlog_init() -> i32 {
    let _ = LOG_SENDER.len();
    INITIALIZED.store(true, Ordering::Relaxed);
    0
}

#[no_mangle]
pub extern "C" fn rlog_reconfigure(xml_ptr: *const c_char) -> i32 {
    rlog_configure(xml_ptr)
}

// ── C shims for simple log calls ──────────────────────────────────────────────

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
            level, message: msg.to_owned(), mdc, ndc: None, logger_name: None,
            thread_name: None, exception: None, timestamp: Utc::now(),
            marker: None, caller_class: None, caller_method: None,
            caller_file: None, caller_line: None,
        });
    }
}

#[no_mangle]
pub extern "C" fn rlog_flush() {
    while !LOG_SENDER.is_empty() {
        thread::sleep(std::time::Duration::from_millis(5));
    }
    thread::sleep(std::time::Duration::from_millis(50));
}

// ── JNI wrappers ─────────────────────────────────────────────────────────────

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::jint;

fn jstring_to_opt(env: &mut JNIEnv, js: &JString) -> Option<String> {
    env.get_string(js).ok().map(String::from).filter(|s| !s.is_empty())
}

fn jstring_to_opt_i32(env: &mut JNIEnv, js: &JString) -> Option<i32> {
    jstring_to_opt(env, js).and_then(|s| s.parse().ok())
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1init(
    _env: JNIEnv, _class: JClass,
) -> jint { rlog_init() }

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1configure(
    mut env: JNIEnv, _class: JClass, xml_content: JString,
) -> jint {
    if let Ok(s) = env.get_string(&xml_content) { rlog_configure(s.as_ptr()) } else { -1 }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1reconfigure(
    mut env: JNIEnv, _class: JClass, xml_content: JString,
) -> jint {
    if let Ok(s) = env.get_string(&xml_content) { rlog_reconfigure(s.as_ptr()) } else { -1 }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log(
    mut env: JNIEnv, _class: JClass, level: jint, message: JString,
) {
    if let Ok(msg) = env.get_string(&message) { rlog_log(level, msg.as_ptr()); }
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1with_1context(
    mut env: JNIEnv, _class: JClass, level: jint, message: JString, context: JString,
) {
    if let Ok(msg) = env.get_string(&message) {
        if let Ok(ctx) = env.get_string(&context) {
            rlog_log_with_context(level, msg.as_ptr(), ctx.as_ptr());
        } else { rlog_log(level, msg.as_ptr()); }
    }
}

/// 7-param JNI call (backward-compatible, no marker/caller).
#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1full(
    mut env: JNIEnv, _class: JClass,
    level: jint, message: JString, mdc: JString, ndc: JString,
    logger_name: JString, thread_name: JString, exception: JString,
) {
    let msg = match env.get_string(&message) { Ok(s) => String::from(s), Err(_) => return };
    send_event(LogEvent {
        level, message: msg,
        mdc:         jstring_to_opt(&mut env, &mdc),
        ndc:         jstring_to_opt(&mut env, &ndc),
        logger_name: jstring_to_opt(&mut env, &logger_name),
        thread_name: jstring_to_opt(&mut env, &thread_name),
        exception:   jstring_to_opt(&mut env, &exception),
        timestamp:   Utc::now(),
        marker:        None,
        caller_class:  None,
        caller_method: None,
        caller_file:   None,
        caller_line:   None,
    });
}

/// 12-param JNI call: adds marker + caller location (Features 8 & 9).
#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1log_1full2(
    mut env: JNIEnv, _class: JClass,
    level: jint, message: JString, mdc: JString, ndc: JString,
    logger_name: JString, thread_name: JString, exception: JString,
    marker: JString, caller_class: JString, caller_method: JString,
    caller_file: JString, caller_line: JString,
) {
    let msg = match env.get_string(&message) { Ok(s) => String::from(s), Err(_) => return };
    send_event(LogEvent {
        level, message: msg,
        mdc:           jstring_to_opt(&mut env, &mdc),
        ndc:           jstring_to_opt(&mut env, &ndc),
        logger_name:   jstring_to_opt(&mut env, &logger_name),
        thread_name:   jstring_to_opt(&mut env, &thread_name),
        exception:     jstring_to_opt(&mut env, &exception),
        timestamp:     Utc::now(),
        marker:        jstring_to_opt(&mut env, &marker),
        caller_class:  jstring_to_opt(&mut env, &caller_class),
        caller_method: jstring_to_opt(&mut env, &caller_method),
        caller_file:   jstring_to_opt(&mut env, &caller_file),
        caller_line:   jstring_to_opt_i32(&mut env, &caller_line),
    });
}

#[no_mangle]
pub extern "system" fn Java_org_apache_logging_log4j_core_NativeLogger_rlog_1flush(
    _env: JNIEnv, _class: JClass,
) { rlog_flush(); }
