package org.apache.logging.log4j.core;

import java.io.InputStream;
import java.util.Scanner;

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

    static {
        // Load the native library using our custom loader
        NativeLoader.load();
        
        try {
            // Attempt to read log4j2.xml from classpath
            InputStream is = NativeLogger.class.getClassLoader().getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner scanner = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    String xmlContent = scanner.hasNext() ? scanner.next() : "";
                    rlog_configure(xmlContent);
                }
            }

            // Initialize the Rust logger
            int status = rlog_init();
            if (status != 0) {
                System.err.println("Rlog4 Warning: Failed to initialize Rust logger, status: " + status);
            }
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
