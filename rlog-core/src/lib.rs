use std::ffi::CStr;
use std::os::raw::c_char;
use std::io::{self, Write};
use crossbeam_channel::{bounded, Sender};
use lazy_static::lazy_static;
use tracing::{error, info, debug, trace, warn};
use std::thread;
use std::sync::Mutex;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::path::Path;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use chrono::Utc;

// ─────────────────────────────────────────────────────────────────────────────
// Enums
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum RollingPolicy { Never, Hourly, Daily }

/// Output format selected by the layout child element inside each appender.
///
/// | log4j2.xml element   | Format  | Example output                                          |
/// |----------------------|---------|----------------------------------------------------------|
/// | `<PatternLayout/>`   | Text    | `2026-05-14T… INFO  Payment OK txnId=abc`               |
/// | `<JsonLayout/>`      | Json    | `{"@timestamp":"…","level":"INFO","message":"Payment OK","txnId":"abc"}` |
/// | `<XmlLayout/>`       | Xml     | `<event timestamp="…" level="INFO"><message>…</message><context>…</context></event>` |
/// | `<LogfmtLayout/>`    | Logfmt  | `ts=… level=INFO msg="Payment OK" txnId=abc`            |
#[derive(Clone, PartialEq)]
enum LogFormat { Text, Json, Xml, Logfmt }

// ─────────────────────────────────────────────────────────────────────────────
// Config (populated by rlog_configure before rlog_init)
// ─────────────────────────────────────────────────────────────────────────────

struct RlogConfig {
    file_name:      Option<String>,
    rolling_policy: RollingPolicy,
    level:          tracing::Level,
    has_console:    bool,
    format:         LogFormat,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name:      None,
        rolling_policy: RollingPolicy::Never,
        level:          tracing::Level::TRACE,
        has_console:    false,
        format:         LogFormat::Text,
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Channel event
// ─────────────────────────────────────────────────────────────────────────────

enum LogEvent {
    Message { level: i32, message: String, context: Option<String> },
    /// Sentinel: drain the queue up to this point then ack on the enclosed channel.
    Flush(crossbeam_channel::Sender<()>),
}

// ─────────────────────────────────────────────────────────────────────────────
// Env-expression resolver  ${env:VAR:-default}
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

// ─────────────────────────────────────────────────────────────────────────────
// Formatting helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Escape a string for embedding inside a JSON double-quoted value.
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

/// Escape a string for embedding inside an XML attribute value or text node.
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

/// Quote a value for logfmt output: wrap in double-quotes if it contains
/// whitespace, `=`, or `"` characters.
fn logfmt_quote(s: &str) -> String {
    if s.chars().any(|c| matches!(c, ' ' | '"' | '=' | '\n' | '\r' | '\t')) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Parse the JSON MDC object produced by `RlogLogger.mdcToJson()`.
///
/// Input:  `{"txnId":"abc123","userId":"xyz"}`
/// Output: `[("txnId", "abc123"), ("userId", "xyz")]`
///
/// Handles standard JSON string escaping. Returns empty vec on malformed input.
fn parse_mdc_json(ctx: &str) -> Vec<(String, String)> {
    fn read_string<'a>(s: &'a str) -> Option<(String, &'a str)> {
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
        let (key, after_key) = match read_string(rest)     { Some(v) => v, None => break };
        let after_colon      = after_key.trim_start_matches([' ', ':']);
        let (val, after_val) = match read_string(after_colon) { Some(v) => v, None => break };
        pairs.push((key, val));
        rest = after_val;
    }
    pairs
}

fn level_name(level: i32) -> &'static str {
    match level { 1 => "TRACE", 2 => "DEBUG", 3 => "INFO", 4 => "WARN", 5 => "ERROR", _ => "INFO" }
}

// ─────────────────────────────────────────────────────────────────────────────
// Writer helpers (used by structured modes)
// ─────────────────────────────────────────────────────────────────────────────

fn make_file_writer(file_name: &str, rolling_policy: &RollingPolicy) -> Box<dyn Write + Send> {
    let path   = Path::new(file_name);
    let dir    = path.parent().unwrap_or(Path::new("."));
    let prefix = path.file_name().unwrap_or_default();
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("rlog: could not create log directory {:?}: {}", dir, e);
    }
    let (nb, guard) = match rolling_policy {
        RollingPolicy::Hourly => tracing_appender::non_blocking(tracing_appender::rolling::hourly(dir, prefix)),
        RollingPolicy::Daily  => tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, prefix)),
        RollingPolicy::Never  => tracing_appender::non_blocking(tracing_appender::rolling::never(dir, prefix)),
    };
    Box::leak(Box::new(guard));
    Box::new(nb)
}

fn write_all_writers(writers: &mut [Box<dyn Write + Send>], line: &str) {
    for w in writers.iter_mut() { let _ = w.write_all(line.as_bytes()); }
}

fn flush_all_writers(writers: &mut [Box<dyn Write + Send>]) {
    for w in writers.iter_mut() { let _ = w.flush(); }
}

// ─────────────────────────────────────────────────────────────────────────────
// Structured-format background loop (JSON / XML / Logfmt)
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
                    // ── JSON ──────────────────────────────────────────────────
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

                    // ── XML ───────────────────────────────────────────────────
                    // <event timestamp="…" level="INFO">
                    //   <message>…</message>
                    //   <context><entry key="txnId" value="abc"/></context>
                    // </event>
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

                    // ── Logfmt ────────────────────────────────────────────────
                    // ts=… level=INFO msg="Payment OK" txnId=abc userId=xyz
                    LogFormat::Logfmt => {
                        let mut s = format!(
                            "ts={ts} level={lvl} msg={}",
                            logfmt_quote(&message)
                        );
                        for (k, v) in &mdc {
                            s.push_str(&format!(" {}={}", k, logfmt_quote(v)));
                        }
                        s.push('\n');
                        s
                    }

                    LogFormat::Text => unreachable!(),
                };

                write_all_writers(&mut writers, &line);
            }

            LogEvent::Flush(ack) => {
                flush_all_writers(&mut writers);
                let _ = ack.send(());
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Background thread + lock-free sender (lazy, initialised on first use)
// ─────────────────────────────────────────────────────────────────────────────

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);

        let (file_name, level, rolling_policy, has_console, format) = {
            let c = CONFIG.lock().unwrap();
            (c.file_name.clone(), c.level, c.rolling_policy.clone(), c.has_console, c.format.clone())
        };

        thread::spawn(move || {
            if format == LogFormat::Text {
                // ── Text mode: tracing-subscriber does timestamps + formatting ──
                let level_filter = tracing_subscriber::filter::LevelFilter::from_level(level);

                if let Some(ref fname) = file_name {
                    let path   = Path::new(fname);
                    let dir    = path.parent().unwrap_or(Path::new("."));
                    let prefix = path.file_name().unwrap_or_default();
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("rlog: could not create log directory {:?}: {}", dir, e);
                    }
                    let (nb, guard) = match rolling_policy {
                        RollingPolicy::Hourly => tracing_appender::non_blocking(tracing_appender::rolling::hourly(dir, prefix)),
                        RollingPolicy::Daily  => tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, prefix)),
                        RollingPolicy::Never  => tracing_appender::non_blocking(tracing_appender::rolling::never(dir, prefix)),
                    };
                    Box::leak(Box::new(guard));
                    if has_console {
                        tracing_subscriber::registry()
                            .with(level_filter)
                            .with(tracing_subscriber::fmt::layer().with_writer(io::stdout))
                            .with(tracing_subscriber::fmt::layer().with_writer(nb))
                            .init();
                    } else {
                        tracing_subscriber::registry()
                            .with(level_filter)
                            .with(tracing_subscriber::fmt::layer().with_writer(nb))
                            .init();
                    }
                } else {
                    tracing_subscriber::fmt().with_max_level(level).init();
                }

                while let Ok(event) = receiver.recv() {
                    match event {
                        LogEvent::Message { level, message, context } => {
                            let full = match context {
                                Some(ctx) => format!("{} {}", message, ctx),
                                None => message,
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
                        LogEvent::Flush(ack) => { let _ = ack.send(()); }
                    }
                }
            } else {
                // ── Structured mode: direct io::Write, format chosen per event ──
                let mut writers: Vec<Box<dyn Write + Send>> = Vec::new();

                if has_console            { writers.push(Box::new(io::stdout())); }
                if let Some(ref fname) = file_name {
                    writers.push(make_file_writer(fname, &rolling_policy));
                }
                if writers.is_empty()     { writers.push(Box::new(io::stdout())); }

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
                    // ── Layout selection ──────────────────────────────────────
                    "jsonlayout"   => { config.format = LogFormat::Json; }
                    "xmllayout"    => { config.format = LogFormat::Xml; }
                    "logfmtlayout" => { config.format = LogFormat::Logfmt; }
                    // <PatternLayout/> or absent → LogFormat::Text (default)

                    // ── Appender types ────────────────────────────────────────
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

                    // ── Root logger level ─────────────────────────────────────
                    "root" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"level" {
                                if let Ok(s) = String::from_utf8(attr.value.into_owned()) {
                                    config.level = match s.to_uppercase().as_str() {
                                        "TRACE"          => tracing::Level::TRACE,
                                        "DEBUG"          => tracing::Level::DEBUG,
                                        "INFO"           => tracing::Level::INFO,
                                        "WARN"           => tracing::Level::WARN,
                                        "ERROR" | "FATAL"=> tracing::Level::ERROR,
                                        _                => tracing::Level::INFO,
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
            let ctx_c = unsafe { CStr::from_ptr(ctx_ptr) };
            ctx_c.to_str().ok().map(|s| s.to_owned())
        } else {
            None
        };
        if let Err(e) = LOG_SENDER.send(LogEvent::Message {
            level,
            message: str_slice.to_owned(),
            context,
        }) {
            if let LogEvent::Message { message, .. } = e.into_inner() {
                eprintln!("rlog: CRITICAL - log channel closed, message dropped: {}", message);
            }
        }
    }
}

/// Block until every message enqueued before this call has been written to disk/stdout,
/// or until the 5-second timeout expires. Called automatically from the JVM shutdown hook.
#[no_mangle]
pub extern "C" fn rlog_flush() {
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    let _ = LOG_SENDER.send(LogEvent::Flush(ack_tx));
    let _ = ack_rx.recv_timeout(std::time::Duration::from_secs(5));
}
