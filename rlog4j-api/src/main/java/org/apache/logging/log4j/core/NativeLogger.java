package org.apache.logging.log4j.core;

import java.io.InputStream;
import java.util.Scanner;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.apache.logging.log4j.Level;

/**
 * Universal Native Bridge for Rlog4 (JNI — Java 8+).
 */
public class NativeLogger {

    // ── Native method declarations ─────────────────────────────────────────

    private static native int  rlog_init();
    private static native int  rlog_configure(String xmlContent);
    private static native void rlog_log(int level, String message);
    private static native void rlog_log_with_context(int level, String message, String context);

    /**
     * Full call: carries MDC, NDC, logger name, thread name, and exception
     * stack trace so all PatternLayout specifiers work correctly.
     */
    private static native void rlog_log_full(int level, String message,
                                             String mdc,
                                             String ndc,
                                             String loggerName,
                                             String threadName,
                                             String exception);

    private static native void rlog_flush();

    /** Runtime reconfiguration — safe to call after rlog_init(). */
    private static native int  rlog_reconfigure(String xmlContent);

    // ── Java-side level tracking for isEnabled() short-circuit ────────────

    private static volatile Level configuredLevel = Level.TRACE;

    /** Per-logger level map: logger-name → effective Level */
    private static final java.util.concurrent.ConcurrentHashMap<String, Level> LOGGER_LEVEL_MAP =
        new java.util.concurrent.ConcurrentHashMap<>();

    private static final Pattern LOOKUP_PATTERN =
        Pattern.compile("\\$\\{(env|sys):([^}:]+)(?::-(.*?))?\\}");
    private static final Pattern ROOT_LEVEL_PATTERN =
        Pattern.compile("<Root[^>]+level=\"([^\"]+)\"", Pattern.CASE_INSENSITIVE);
    // Matches <Logger ...> or <Logger ... /> — captures the attribute block
    private static final Pattern LOGGER_ELEMENT_PATTERN =
        Pattern.compile("<Logger\\b([^>]+?)(?:/>|>)", Pattern.CASE_INSENSITIVE | Pattern.DOTALL);
    private static final Pattern ATTR_NAME_PATTERN =
        Pattern.compile("\\bname=\"([^\"]+)\"", Pattern.CASE_INSENSITIVE);
    private static final Pattern ATTR_LEVEL_PATTERN =
        Pattern.compile("\\blevel=\"([^\"]+)\"", Pattern.CASE_INSENSITIVE);

    // ── Static initialiser ─────────────────────────────────────────────────

    static {
        NativeLoader.load();

        try {
            String xmlContent = null;
            ClassLoader cl = NativeLogger.class.getClassLoader();

            // XML
            InputStream is = cl.getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner sc = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    xmlContent = sc.hasNext() ? sc.next() : "";
                }
            }

            // YAML / YML
            if (xmlContent == null) {
                is = cl.getResourceAsStream("log4j2.yaml");
                if (is == null) is = cl.getResourceAsStream("log4j2.yml");
                if (is != null) {
                    try (Scanner sc = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                        xmlContent = ConfigConverter.yamlToXml(sc.hasNext() ? sc.next() : "");
                    }
                }
            }

            // JSON
            if (xmlContent == null) {
                is = cl.getResourceAsStream("log4j2.json");
                if (is != null) {
                    try (Scanner sc = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                        xmlContent = ConfigConverter.jsonToXml(sc.hasNext() ? sc.next() : "");
                    }
                }
            }

            if (xmlContent != null) {
                xmlContent = resolveLookups(xmlContent);
                parseConfiguredLevel(xmlContent);
                parseLoggerLevels(xmlContent);
                System.out.println("Rlog4 Config XML:\n" + xmlContent);
                rlog_configure(xmlContent);
            }

            int status = rlog_init();
            if (status != 0) {
                System.err.println("Rlog4 Warning: Failed to initialise Rust logger, status=" + status);
            }

            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                try {
                    System.out.println("Rlog4: JVM shutting down, flushing async queues…");
                    rlog_flush();
                } catch (Throwable t) {
                    System.err.println("Rlog4 Warning: Failed to flush logs on shutdown");
                }
            }));

        } catch (Throwable t) {
            System.err.println("Rlog4 Error: Critical failure during native initialisation");
            t.printStackTrace();
        }
    }

    // ── Lookup resolution ──────────────────────────────────────────────────

    static String resolveLookups(String xml) {
        if (xml == null) return null;
        Matcher m = LOOKUP_PATTERN.matcher(xml);
        StringBuffer sb = new StringBuffer();
        while (m.find()) {
            String type       = m.group(1);
            String key        = m.group(2);
            String defaultVal = m.group(3);

            String resolved = "env".equals(type) ? System.getenv(key)
                                                  : System.getProperty(key);
            if (resolved == null) resolved = (defaultVal != null) ? defaultVal : "";
            m.appendReplacement(sb, Matcher.quoteReplacement(resolved));
        }
        m.appendTail(sb);
        return sb.toString();
    }

    private static void parseConfiguredLevel(String xml) {
        Matcher m = ROOT_LEVEL_PATTERN.matcher(xml);
        if (m.find()) {
            configuredLevel = parseLevel(m.group(1));
        }
    }

    /** Parse all &lt;Logger name="..." level="..."/&gt; entries into LOGGER_LEVEL_MAP. */
    static void parseLoggerLevels(String xml) {
        LOGGER_LEVEL_MAP.clear();
        Matcher em = LOGGER_ELEMENT_PATTERN.matcher(xml);
        while (em.find()) {
            String attrs = em.group(1);
            Matcher nm = ATTR_NAME_PATTERN.matcher(attrs);
            Matcher lm = ATTR_LEVEL_PATTERN.matcher(attrs);
            if (nm.find() && lm.find()) {
                String name  = nm.group(1).trim();
                Level  level = parseLevel(lm.group(1).trim());
                LOGGER_LEVEL_MAP.put(name, level);
                System.out.println("Rlog4 Java: Logger '" + name + "' → " + level);
            }
        }
    }

    private static Level parseLevel(String s) {
        switch (s.toUpperCase()) {
            case "TRACE": return Level.TRACE;
            case "DEBUG": return Level.DEBUG;
            case "INFO":  return Level.INFO;
            case "WARN":  return Level.WARN;
            case "ERROR":
            case "FATAL": return Level.ERROR;
            default:      return Level.INFO;
        }
    }

    /**
     * Resolves the effective level for a logger by walking the dot-separated
     * hierarchy from most-specific to root, mirroring Rust-side logic so that
     * RlogLogger.isEnabled() can short-circuit without a JNI call.
     */
    public static Level getEffectiveLevel(String loggerName) {
        if (loggerName == null || loggerName.isEmpty()) return configuredLevel;
        String name = loggerName;
        while (!name.isEmpty()) {
            Level l = LOGGER_LEVEL_MAP.get(name);
            if (l != null) return l;
            int dot = name.lastIndexOf('.');
            if (dot < 0) break;
            name = name.substring(0, dot);
        }
        return configuredLevel;
    }

    public static Level getConfiguredLevel() { return configuredLevel; }

    // ── Runtime reconfiguration ────────────────────────────────────────────────

    private static volatile Thread watcherThread = null;
    private static volatile boolean watcherRunning = false;

    /**
     * Reload the native Rust logger from an XML string and update Java-side
     * level maps immediately. Safe to call from any thread at any time.
     */
    public static void reconfigureFromXml(String xmlContent) {
        if (xmlContent == null || xmlContent.isEmpty()) return;
        String resolved = resolveLookups(xmlContent);
        parseConfiguredLevel(resolved);
        parseLoggerLevels(resolved);
        try {
            rlog_reconfigure(resolved);
        } catch (Throwable t) {
            System.err.println("Rlog4 Warning: rlog_reconfigure failed: " + t.getMessage());
        }
    }

    /**
     * Reload from a file path. Reads the file content and delegates to
     * {@link #reconfigureFromXml(String)}.
     */
    public static void reconfigureFromFile(String configFilePath) {
        try {
            String xml = new java.util.Scanner(
                    new java.io.File(configFilePath), "UTF-8")
                    .useDelimiter("\\A").next();
            reconfigureFromXml(xml);
            System.out.println("Rlog4: reloaded config from " + configFilePath);
        } catch (Throwable t) {
            System.err.println("Rlog4 Warning: cannot read config file " + configFilePath + ": " + t.getMessage());
        }
    }

    /**
     * Start a background thread that watches {@code configFilePath} for changes
     * and calls {@link #reconfigureFromFile(String)} automatically.
     * Only one watcher runs at a time; calling this again replaces the previous one.
     */
    public static synchronized void startFileWatcher(String configFilePath) {
        stopFileWatcher();
        watcherRunning = true;
        watcherThread = new Thread(() -> {
            try {
                java.nio.file.Path path = java.nio.file.Paths.get(configFilePath);
                java.nio.file.Path dir  = path.getParent();
                if (dir == null) dir = java.nio.file.Paths.get(".");
                try (java.nio.file.WatchService watcher =
                         java.nio.file.FileSystems.getDefault().newWatchService()) {
                    dir.register(watcher,
                            java.nio.file.StandardWatchEventKinds.ENTRY_MODIFY);
                    System.out.println("Rlog4: watching " + configFilePath + " for changes");
                    while (watcherRunning) {
                        java.nio.file.WatchKey key = watcher.poll(500, java.util.concurrent.TimeUnit.MILLISECONDS);
                        if (key == null) continue;
                        for (java.nio.file.WatchEvent<?> event : key.pollEvents()) {
                            Object ctx = event.context();
                            if (ctx != null && ctx.toString().equals(path.getFileName().toString())) {
                                Thread.sleep(100); // let the write finish
                                reconfigureFromFile(configFilePath);
                            }
                        }
                        if (!key.reset()) break;
                    }
                }
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            } catch (Throwable t) {
                System.err.println("Rlog4 Warning: file watcher error: " + t.getMessage());
            }
        }, "rlog4-file-watcher");
        watcherThread.setDaemon(true);
        watcherThread.start();
    }

    /** Stop the file-watcher thread if it is running. */
    public static synchronized void stopFileWatcher() {
        watcherRunning = false;
        if (watcherThread != null) {
            watcherThread.interrupt();
            watcherThread = null;
        }
    }

    // ── Public log API ─────────────────────────────────────────────────────

    /** Convenience — no metadata (used by tests and benchmark). */
    public static void log(int level, String message) {
        log(level, message, null, null, null, null, null);
    }

    /** Convenience — MDC only. */
    public static void log(int level, String message, String mdcString) {
        log(level, message, mdcString, null, null, null, null);
    }

    /** Convenience — MDC + logger/thread names, no NDC, no exception. */
    public static void log(int level, String message,
                           String mdcString, String loggerName, String threadName) {
        log(level, message, mdcString, null, loggerName, threadName, null);
    }

    /**
     * Full call used by {@link RlogLogger}.
     * Passes MDC, NDC, logger name, thread name, and exception stack trace so
     * all PatternLayout specifiers (%X, %x, %logger, %t, %ex) work correctly.
     */
    public static void log(int level, String message,
                           String mdcString, String ndcString,
                           String loggerName, String threadName,
                           String exception) {
        try {
            rlog_log_full(
                level,
                message,
                mdcString  != null ? mdcString  : "",
                ndcString  != null ? ndcString  : "",
                loggerName != null ? loggerName : "",
                threadName != null ? threadName : Thread.currentThread().getName(),
                exception  != null ? exception  : ""
            );
        } catch (Throwable t) {
            System.err.println("Rlog4 Native Log Failure: " + message);
        }
    }
}
