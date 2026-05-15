package org.apache.logging.log4j.core;

import java.io.InputStream;
import java.util.Scanner;
import org.apache.logging.log4j.Level;

/**
 * Java 11 JNI bridge — used on Java 8–21.
 * Java 22+ loads the FFM override from META-INF/versions/22/ instead.
 */
public class NativeLogger {

    static volatile Level configuredLevel = Level.INFO;

    static {
        try {
            InputStream is = NativeLogger.class.getClassLoader()
                    .getResourceAsStream("log4j2.xml");
            if (is != null) {
                try (Scanner scanner = new Scanner(is, "UTF-8").useDelimiter("\\A")) {
                    String xml = scanner.hasNext() ? scanner.next() : "";
                    configuredLevel = parseRootLevel(xml);
                    NativeLoggerJNI.rlogConfigure(xml);
                }
            }

            int status = NativeLoggerJNI.rlogInit();
            if (status != 0) {
                throw new RuntimeException("Failed to initialize Rust logger, status: " + status);
            }

            Runtime.getRuntime().addShutdownHook(new Thread(() ->
                NativeLoggerJNI.rlogFlush(), "rlog-shutdown"));

        } catch (Throwable t) {
            throw new ExceptionInInitializerError(t);
        }
    }

    private static Level parseRootLevel(String xml) {
        int rootIdx = xml.indexOf("<Root");
        if (rootIdx < 0) return Level.INFO;
        int gt = xml.indexOf('>', rootIdx);
        int levelIdx = xml.indexOf("level=", rootIdx);
        if (levelIdx < 0 || levelIdx > gt) return Level.INFO;
        char quote = xml.charAt(levelIdx + 6);
        int start = levelIdx + 7;
        int end = xml.indexOf(quote, start);
        if (end < 0) return Level.INFO;
        return Level.toLevel(xml.substring(start, end), Level.INFO);
    }

    public static void log(int level, String message) {
        log(level, message, null);
    }

    public static void log(int level, String message, String mdcString) {
        NativeLoggerJNI.rlogLog(level, message, mdcString);
    }
}
