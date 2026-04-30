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

#[derive(Clone, PartialEq)]
enum RollingPolicy {
    Never,
    Hourly,
    Daily,
}

// Configuration state
struct RlogConfig {
    file_name: Option<String>,
    rolling_policy: RollingPolicy,
    level: tracing::Level,
}

lazy_static! {
    static ref CONFIG: Mutex<RlogConfig> = Mutex::new(RlogConfig {
        file_name: None,
        rolling_policy: RollingPolicy::Never,
        level: tracing::Level::TRACE,
    });
}

// Define a log event struct
struct LogEvent {
    level: i32,
    message: String,
    context: Option<String>,
}

lazy_static! {
    static ref LOG_SENDER: Sender<LogEvent> = {
        let (sender, receiver) = bounded::<LogEvent>(1_000_000);
        
        // Read configuration and extract values to move into the thread
        let (file_name, level, rolling_policy) = {
            let config = CONFIG.lock().unwrap();
            (config.file_name.clone(), config.level, config.rolling_policy.clone())
        };
        
        thread::spawn(move || {
            // Set up tracing subscriber
            let subscriber_builder = tracing_subscriber::fmt()
                .with_max_level(level);
            
            if let Some(file_name) = &file_name {
                let path = Path::new(file_name);
                let dir = path.parent().unwrap_or(Path::new("."));
                let file_prefix = path.file_name().unwrap_or_default();
                
                let (non_blocking, _guard) = match rolling_policy {
                    RollingPolicy::Hourly => {
                        let appender = tracing_appender::rolling::hourly(dir, file_prefix);
                        tracing_appender::non_blocking(appender)
                    },
                    RollingPolicy::Daily => {
                        let appender = tracing_appender::rolling::daily(dir, file_prefix);
                        tracing_appender::non_blocking(appender)
                    },
                    RollingPolicy::Never => {
                        let appender = tracing_appender::rolling::never(dir, file_prefix);
                        tracing_appender::non_blocking(appender)
                    }
                };

                subscriber_builder.with_writer(non_blocking).init();
                Box::leak(Box::new(_guard));
            } else {
                subscriber_builder.init();
            }
            
            while let Ok(event) = receiver.recv() {
                // If context is present, append it to the message
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
    if let Ok(xml_str) = c_str.to_str() {
        println!("Rust received XML length: {}", xml_str.len());
        let mut reader = Reader::from_str(xml_str);
        reader.trim_text(true);
        
        let mut config = CONFIG.lock().unwrap();
        
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                    let name = e.name();
                    let name_str = String::from_utf8_lossy(name.as_ref());
                    
                    if name_str.eq_ignore_ascii_case("File") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"fileName" {
                                    if let Ok(file_name) = String::from_utf8(attr.value.into_owned()) {
                                        println!("Rust extracted file name: {}", file_name);
                                        config.file_name = Some(file_name);
                                        config.rolling_policy = RollingPolicy::Never;
                                    }
                                }
                            }
                        }
                    } else if name_str.eq_ignore_ascii_case("RollingFile") {
                        let mut file_name = None;
                        let mut policy = RollingPolicy::Never;
                        
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"fileName" {
                                    if let Ok(f_name) = String::from_utf8(attr.value.into_owned()) {
                                        file_name = Some(f_name);
                                    }
                                } else if attr.key.as_ref() == b"filePattern" {
                                    if let Ok(pattern) = String::from_utf8(attr.value.into_owned()) {
                                        if pattern.contains("HH") {
                                            policy = RollingPolicy::Hourly;
                                        } else if pattern.contains("%d{") {
                                            policy = RollingPolicy::Daily;
                                        }
                                    }
                                }
                            }
                        }
                        
                        if let Some(f_name) = file_name {
                            println!("Rust extracted rolling file name: {:?}", f_name);
                            config.file_name = Some(f_name);
                            config.rolling_policy = policy;
                        }
                    } else if name_str.eq_ignore_ascii_case("Root") {
                        for attr in e.attributes() {
                            if let Ok(attr) = attr {
                                if attr.key.as_ref() == b"level" {
                                    if let Ok(level_str) = String::from_utf8(attr.value.into_owned()) {
                                        config.level = match level_str.to_uppercase().as_str() {
                                            "TRACE" => tracing::Level::TRACE,
                                            "DEBUG" => tracing::Level::DEBUG,
                                            "INFO" => tracing::Level::INFO,
                                            "WARN" => tracing::Level::WARN,
                                            "ERROR" | "FATAL" => tracing::Level::ERROR,
                                            _ => tracing::Level::INFO,
                                        };
                                    }
                                }
                            }
                        }
                    }
                },
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => (),
            }
            buf.clear();
        }
        return 0; // Success
    }
    
    -1 // Error
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
        let mut context = None;
        if !ctx_ptr.is_null() {
            let ctx_str = unsafe { CStr::from_ptr(ctx_ptr) };
            if let Ok(ctx_slice) = ctx_str.to_str() {
                context = Some(ctx_slice.to_owned());
            }
        }

        let _ = LOG_SENDER.try_send(LogEvent {
            level,
            message: str_slice.to_owned(),
            context,
        });
    }
}
