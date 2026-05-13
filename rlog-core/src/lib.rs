use std::ffi::CStr;
use std::os::raw::c_char;
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

#[derive(Clone, PartialEq)]
enum RollingPolicy {
    Never,
    Hourly,
    Daily,
}

struct RlogConfig {
    file_name: Option<String>,
    rolling_policy: RollingPolicy,
    level: tracing::Level,
    has_console: bool,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name: None,
        rolling_policy: RollingPolicy::Never,
        level: tracing::Level::TRACE,
        has_console: false,
    });
}

struct LogEvent {
    level: i32,
    message: String,
    context: Option<String>,
}

/// Resolve Log4j2-style `${env:VAR:-default}` property expressions using the process environment.
fn resolve_env_expr(s: &str) -> String {
    let mut result = s.to_string();
    loop {
        let start = match result.find("${") {
            Some(i) => i,
            None => break,
        };
        let end = match result[start..].find('}') {
            Some(i) => start + i,
            None => break,
        };
        let expr = &result[start + 2..end]; // content between ${ and }
        let resolved = if let Some(rest) = expr.strip_prefix("env:") {
            let (var_name, default) = match rest.find(":-") {
                Some(d) => (&rest[..d], &rest[d + 2..]),
                None => (rest, ""),
            };
            std::env::var(var_name).unwrap_or_else(|_| default.to_string())
        } else {
            // Unknown expression type — leave as-is to avoid infinite loop
            break;
        };
        result = format!("{}{}{}", &result[..start], resolved, &result[end + 1..]);
    }
    result
}

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);

        let (file_name, level, rolling_policy, has_console) = {
            let config = CONFIG.lock().unwrap();
            (config.file_name.clone(), config.level, config.rolling_policy.clone(), config.has_console)
        };

        thread::spawn(move || {
            let level_filter = tracing_subscriber::filter::LevelFilter::from_level(level);

            if let Some(ref file_name) = file_name {
                let path = Path::new(file_name);
                let dir = path.parent().unwrap_or(Path::new("."));
                let file_prefix = path.file_name().unwrap_or_default();

                if let Err(e) = std::fs::create_dir_all(dir) {
                    eprintln!("rlog: could not create log directory {:?}: {}", dir, e);
                }

                let (non_blocking, guard) = match rolling_policy {
                    RollingPolicy::Hourly => tracing_appender::non_blocking(tracing_appender::rolling::hourly(dir, file_prefix)),
                    RollingPolicy::Daily  => tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, file_prefix)),
                    RollingPolicy::Never  => tracing_appender::non_blocking(tracing_appender::rolling::never(dir, file_prefix)),
                };
                Box::leak(Box::new(guard));

                // A single global filter on the registry applies to all layers,
                // avoiding per-layer generic type conflicts.
                if has_console {
                    tracing_subscriber::registry()
                        .with(level_filter)
                        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stdout))
                        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
                        .init();
                } else {
                    tracing_subscriber::registry()
                        .with(level_filter)
                        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
                        .init();
                }
            } else {
                // No file appender — stdout only
                tracing_subscriber::fmt()
                    .with_max_level(level)
                    .init();
            }

            while let Ok(event) = receiver.recv() {
                let full_message = match event.context {
                    Some(ctx) => format!("{} {}", event.message, ctx),
                    None => event.message,
                };
                match event.level {
                    1 => trace!("{}", full_message),
                    2 => debug!("{}", full_message),
                    3 => info!("{}", full_message),
                    4 => warn!("{}", full_message),
                    5 => error!("{}", full_message),
                    _ => info!("{}", full_message),
                }
            }
        });

        sender
    };
}

#[no_mangle]
pub extern "C" fn rlog_configure(xml_ptr: *const c_char) -> i32 {
    if xml_ptr.is_null() {
        return -1;
    }

    let c_str = unsafe { CStr::from_ptr(xml_ptr) };
    let Ok(xml_str) = c_str.to_str() else { return -1; };

    let mut reader = Reader::from_str(xml_str);
    reader.trim_text(true);

    let mut config = CONFIG.lock().unwrap();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let name_str = String::from_utf8_lossy(e.name().as_ref()).to_lowercase();

                match name_str.as_str() {
                    "console" => {
                        config.has_console = true;
                    }
                    "file" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"fileName" {
                                if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                    let resolved = resolve_env_expr(&raw);
                                    config.file_name = Some(resolved);
                                    config.rolling_policy = RollingPolicy::Never;
                                }
                            }
                        }
                    }
                    "rollingfile" => {
                        let mut file_name = None;
                        let mut policy = RollingPolicy::Never;
                        for attr in e.attributes().flatten() {
                            match attr.key.as_ref() {
                                b"fileName" => {
                                    if let Ok(raw) = String::from_utf8(attr.value.into_owned()) {
                                        file_name = Some(resolve_env_expr(&raw));
                                    }
                                }
                                b"filePattern" => {
                                    if let Ok(pattern) = String::from_utf8(attr.value.into_owned()) {
                                        if pattern.contains("HH") {
                                            policy = RollingPolicy::Hourly;
                                        } else if pattern.contains("%d{") {
                                            policy = RollingPolicy::Daily;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if let Some(f) = file_name {
                            config.file_name = Some(f);
                            config.rolling_policy = policy;
                        }
                    }
                    "root" => {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"level" {
                                if let Ok(level_str) = String::from_utf8(attr.value.into_owned()) {
                                    config.level = match level_str.to_uppercase().as_str() {
                                        "TRACE" => tracing::Level::TRACE,
                                        "DEBUG" => tracing::Level::DEBUG,
                                        "INFO"  => tracing::Level::INFO,
                                        "WARN"  => tracing::Level::WARN,
                                        "ERROR" | "FATAL" => tracing::Level::ERROR,
                                        _ => tracing::Level::INFO,
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
    if msg_ptr.is_null() {
        return;
    }

    let c_str = unsafe { CStr::from_ptr(msg_ptr) };
    if let Ok(str_slice) = c_str.to_str() {
        let context = if !ctx_ptr.is_null() {
            let ctx_c = unsafe { CStr::from_ptr(ctx_ptr) };
            ctx_c.to_str().ok().map(|s| s.to_owned())
        } else {
            None
        };

        let _ = LOG_SENDER.try_send(LogEvent {
            level,
            message: str_slice.to_owned(),
            context,
        });
    }
}
