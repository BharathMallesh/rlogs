package org.apache.logging.log4j.core;

import java.io.InputStream;
import java.util.Scanner;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Universal Native Bridge for Rlog4.
 * This version uses JNI (Java Native Interface) instead of FFM (Java 22)
 * to support Java 8, 11, 17, and 22+.
 */
public class NativeLogger {

    // Native method declarations
    private static native int rlog_init();
    private static native int rlog_configure(String xmlContent);
    private static native void rlog_log(int level, String message);
    private static native void rlog_log_with_context(int level, String message, String context);
    private static native void rlog_flush();

    // Pattern: ${env:KEY}, ${sys:KEY}, ${env:KEY:-default}, ${sys:KEY:-default}
    private static final Pattern LOOKUP_PATTERN = Pattern.compile("\\$\\{(env|sys):([^}:]+)(?::-(.*?))?\\}");

    /**
     * Resolves Log4j2-style lookups in the XML configuration string.
     * Supports ${env:VAR_NAME}, ${sys:prop.name}, and default values via :-
     */
    static String resolveLookups(String xml) {
        if (xml == null) return null;
        Matcher matcher = LOOKUP_PATTERN.matcher(xml);
        StringBuffer sb = new StringBuffer();
        while (matcher.find()) {
            String type = matcher.group(1);       // "env" or "sys"
            String key = matcher.group(2);         // variable name
            String defaultVal = matcher.group(3);  // default value (nullable)

            String resolved = null;
            if ("env".equals(type)) {
                resolved = System.getenv(key);
            } else if ("sys".equals(type)) {
                resolved = System.getProperty(key);
            }

            if (resolved == null) {
                resolved = (defaultVal != null) ? defaultVal : "";
            }

            matcher.appendReplacement(sb, Matcher.quoteReplacement(resolved));
        }
        matcher.appendTail(sb);
        return sb.toString();
    }

    static {
        // Load the native library using our custom loader
        NativeLoader.load();
        
        try {
            // Attempt to read log4j2.xml from classpath
            InputStream is = NativeLogger.class.getClassLoader().getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner scanner = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    String xmlContent = scanner.hasNext() ? scanner.next() : "";
                    // Resolve ${env:...} and ${sys:...} lookups before passing to Rust
                    xmlContent = resolveLookups(xmlContent);
                    rlog_configure(xmlContent);
                }
            }

            int status = rlog_init();
            if (status != 0) {
                System.err.println("Rlog4 Warning: Failed to initialize Rust logger, status: " + status);
            }

            // Register Shutdown Hook to flush logs on JVM exit
            Runtime.getRuntime().addShutdownHook(new Thread(() -> {
                try {
                    System.out.println("Rlog4: JVM shutting down, flushing async queues...");
                    rlog_flush();
                } catch (Throwable t) {
                    System.err.println("Rlog4 Warning: Failed to flush logs on shutdown");
                }
            }));
        } catch (Throwable t) {
            System.err.println("Rlog4 Error: Critical failure during native initialization");
            t.printStackTrace();
        }
    }

    public static void log(int level, String message) {
        log(level, message, null);
    }

    public static void log(int level, String message, String mdcString) {
        try {
            if (mdcString == null || mdcString.isEmpty()) {
                rlog_log(level, message);
            } else {
                rlog_log_with_context(level, message, mdcString);
            }
        } catch (Throwable t) {
            // Fallback to prevent app crash if native call fails
            System.err.println("Rlog4 Native Log Failure: " + message);
        }
    }
}
