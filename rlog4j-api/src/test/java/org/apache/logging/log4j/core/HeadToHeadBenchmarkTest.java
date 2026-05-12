package org.apache.logging.log4j.core;

import org.apache.logging.log4j.Logger;
import org.junit.jupiter.api.Test;

import java.io.File;
import java.net.URI;
import java.nio.file.Files;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Head-to-head throughput comparison:
 *   rlog4 (FFM, JNI direct, full Logger API path)
 *   Log4j2 sync     (FileAppender via log4j-core)
 *   Log4j2 async    (Disruptor → FileAppender via AsyncLoggerContextSelector)
 *
 * Same pattern, same file appender, same 500_000 events, single thread,
 * 50_000 warmup. We construct Log4j2's LoggerContext directly so the
 * service-loader fight with rlog4j-api is bypassed.
 */
public class HeadToHeadBenchmarkTest {

    private static final int WARMUP = 50_000;
    private static final int EVENTS = 500_000;
    private static final String PATTERN =
        "%d{yyyy-MM-dd HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n";

    @Test
    public void headToHead() throws Exception {
        System.out.println("\n=========== HEAD-TO-HEAD: rlog4 vs Log4j2 ===========\n");
        System.out.println("warmup = " + WARMUP + " events, measured = " + EVENTS + " events");
        System.out.println("single thread, file appender, pattern: " + PATTERN);
        System.out.println();

        Map<String, Long> results = new LinkedHashMap<>();
        Map<String, Long> sizes   = new LinkedHashMap<>();

        // ── 1. rlog4 FFM (the fast path) ─────────────────────────────────
        File rlogFfmFile = File.createTempFile("rlog4-ffm-", ".log");
        rlogFfmFile.deleteOnExit();
        long rlogFfm = benchRlog4Ffm(rlogFfmFile);
        results.put("rlog4 FFM (NativeLoggerFFM.log)", rlogFfm);
        sizes.put  ("rlog4 FFM (NativeLoggerFFM.log)", rlogFfmFile.length());

        // ── 2. rlog4 JNI direct (NativeLogger.log) ────────────────────────
        File rlogJniFile = File.createTempFile("rlog4-jni-", ".log");
        rlogJniFile.deleteOnExit();
        long rlogJni = benchRlog4Jni(rlogJniFile);
        results.put("rlog4 JNI direct (NativeLogger.log)", rlogJni);
        sizes.put  ("rlog4 JNI direct (NativeLogger.log)", rlogJniFile.length());

        // ── 3. Log4j2 sync (real log4j-core, LoggerContext directly) ──────
        File log4jSyncFile = File.createTempFile("log4j2-sync-", ".log");
        log4jSyncFile.deleteOnExit();
        long log4jSync = benchLog4j2Sync(log4jSyncFile);
        results.put("Log4j2 sync (log4j-core FileAppender)", log4jSync);
        sizes.put  ("Log4j2 sync (log4j-core FileAppender)", log4jSyncFile.length());

        // ── 4. Log4j2 async (Disruptor) ───────────────────────────────────
        File log4jAsyncFile = File.createTempFile("log4j2-async-", ".log");
        log4jAsyncFile.deleteOnExit();
        long log4jAsync = benchLog4j2Async(log4jAsyncFile);
        results.put("Log4j2 async (Disruptor → file)", log4jAsync);
        sizes.put  ("Log4j2 async (Disruptor → file)", log4jAsyncFile.length());

        // ── Report ─────────────────────────────────────────────────────────
        System.out.println();
        System.out.println(repeat('═', 78));
        System.out.println("RESULTS — events/sec (single thread, producer rate)");
        System.out.println(repeat('═', 78));
        long max = results.values().stream().mapToLong(Long::longValue).max().orElse(1);
        for (Map.Entry<String, Long> e : results.entrySet()) {
            double pct = 100.0 * e.getValue() / max;
            int bars   = (int) (pct / 2.0); // 50-char bar
            System.out.printf("%-42s %,12d ev/s  %s%n",
                e.getKey(), e.getValue(), repeat('█', bars));
        }
        System.out.println(repeat('═', 78));
        System.out.println();
        System.out.println("File sizes on disk (sanity check that events actually landed):");
        for (Map.Entry<String, Long> e : sizes.entrySet()) {
            System.out.printf("  %-42s %,12d bytes%n", e.getKey(), e.getValue());
        }
        System.out.println();
    }

    // ── rlog4 FFM ────────────────────────────────────────────────────────

    private long benchRlog4Ffm(File logFile) throws Exception {
        Configurator.reconfigure(rlogXml(logFile));
        Thread.sleep(300);

        System.out.println("[rlog4 FFM] warmup...");
        for (int i = 0; i < WARMUP; i++) {
            NativeLoggerFFM.log(3, "Warmup event " + i);
        }
        Thread.sleep(300);

        System.out.println("[rlog4 FFM] measuring...");
        long start = System.nanoTime();
        for (int i = 0; i < EVENTS; i++) {
            NativeLoggerFFM.log(3, "Benchmark event message number " + i);
        }
        long elapsed = System.nanoTime() - start;
        Thread.sleep(1000);
        long eps = (long) (EVENTS / (elapsed / 1_000_000_000.0));
        System.out.printf("[rlog4 FFM] %,d events in %.3fs → %,d ev/s%n%n",
                EVENTS, elapsed / 1_000_000_000.0, eps);
        return eps;
    }

    // ── rlog4 JNI direct ─────────────────────────────────────────────────

    private long benchRlog4Jni(File logFile) throws Exception {
        Configurator.reconfigure(rlogXml(logFile));
        Thread.sleep(300);

        System.out.println("[rlog4 JNI] warmup...");
        for (int i = 0; i < WARMUP; i++) {
            NativeLogger.log(3, "Warmup event " + i);
        }
        Thread.sleep(300);

        System.out.println("[rlog4 JNI] measuring...");
        long start = System.nanoTime();
        for (int i = 0; i < EVENTS; i++) {
            NativeLogger.log(3, "Benchmark event message number " + i);
        }
        long elapsed = System.nanoTime() - start;
        Thread.sleep(1000);
        long eps = (long) (EVENTS / (elapsed / 1_000_000_000.0));
        System.out.printf("[rlog4 JNI] %,d events in %.3fs → %,d ev/s%n%n",
                EVENTS, elapsed / 1_000_000_000.0, eps);
        return eps;
    }

    // ── Log4j2 sync (real log4j-core) ────────────────────────────────────

    private long benchLog4j2Sync(File logFile) throws Exception {
        File cfg = writeTempConfig(log4jXml(logFile, /*async*/ false));

        // Bypass LogManager (rlog4 owns it) — construct a LoggerContext directly.
        URI cfgUri = cfg.toURI();
        org.apache.logging.log4j.core.LoggerContext ctx =
            new org.apache.logging.log4j.core.LoggerContext("BenchSync", null, cfgUri);
        ctx.start();
        try {
            Logger logger = ctx.getLogger("com.benchmark.Log4j2Sync");

            System.out.println("[Log4j2 sync] warmup...");
            for (int i = 0; i < WARMUP; i++) {
                logger.info("Warmup event " + i);
            }

            System.out.println("[Log4j2 sync] measuring...");
            long start = System.nanoTime();
            for (int i = 0; i < EVENTS; i++) {
                logger.info("Benchmark event message number " + i);
            }
            long elapsed = System.nanoTime() - start;
            long eps = (long) (EVENTS / (elapsed / 1_000_000_000.0));
            System.out.printf("[Log4j2 sync] %,d events in %.3fs → %,d ev/s%n%n",
                    EVENTS, elapsed / 1_000_000_000.0, eps);
            return eps;
        } finally {
            ctx.stop();
            cfg.delete();
        }
    }

    // ── Log4j2 async via Disruptor (AsyncAppender path) ──────────────────

    private long benchLog4j2Async(File logFile) throws Exception {
        File cfg = writeTempConfig(log4jXml(logFile, /*async*/ true));

        URI cfgUri = cfg.toURI();
        org.apache.logging.log4j.core.LoggerContext ctx =
            new org.apache.logging.log4j.core.LoggerContext("BenchAsync", null, cfgUri);
        ctx.start();
        try {
            Logger logger = ctx.getLogger("com.benchmark.Log4j2Async");

            System.out.println("[Log4j2 async] warmup...");
            for (int i = 0; i < WARMUP; i++) {
                logger.info("Warmup event " + i);
            }
            Thread.sleep(300);

            System.out.println("[Log4j2 async] measuring...");
            long start = System.nanoTime();
            for (int i = 0; i < EVENTS; i++) {
                logger.info("Benchmark event message number " + i);
            }
            long elapsed = System.nanoTime() - start;
            Thread.sleep(1500); // let Disruptor drain
            long eps = (long) (EVENTS / (elapsed / 1_000_000_000.0));
            System.out.printf("[Log4j2 async] %,d events in %.3fs → %,d ev/s%n%n",
                    EVENTS, elapsed / 1_000_000_000.0, eps);
            return eps;
        } finally {
            ctx.stop();
            cfg.delete();
        }
    }

    // ── XML helpers ──────────────────────────────────────────────────────

    private static String rlogXml(File logFile) {
        // AsyncQueueFullPolicy="Block" matches Log4j2's default async behaviour
        // (the producer blocks rather than dropping events when the channel fills).
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <AsyncQueueFullPolicy type=\"Block\"/>\n"
                + "  <Appenders>\n"
                + "    <File name=\"BenchFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"" + PATTERN + "\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"BenchFile\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }

    private static String log4jXml(File logFile, boolean async) {
        String appenderBlock;
        String loggerBlock;
        if (async) {
            appenderBlock =
                "    <File name=\"BenchFile\" fileName=\"" + logFile.getAbsolutePath() + "\" append=\"false\">\n"
              + "      <PatternLayout pattern=\"" + PATTERN + "\"/>\n"
              + "    </File>\n"
              + "    <Async name=\"AsyncBench\" bufferSize=\"1048576\">\n"
              + "      <AppenderRef ref=\"BenchFile\"/>\n"
              + "    </Async>\n";
            loggerBlock =
                "    <Root level=\"INFO\"><AppenderRef ref=\"AsyncBench\"/></Root>\n";
        } else {
            appenderBlock =
                "    <File name=\"BenchFile\" fileName=\"" + logFile.getAbsolutePath() + "\" append=\"false\">\n"
              + "      <PatternLayout pattern=\"" + PATTERN + "\"/>\n"
              + "    </File>\n";
            loggerBlock =
                "    <Root level=\"INFO\"><AppenderRef ref=\"BenchFile\"/></Root>\n";
        }
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
             + "<Configuration status=\"WARN\">\n"
             + "  <Appenders>\n" + appenderBlock + "  </Appenders>\n"
             + "  <Loggers>\n"   + loggerBlock   + "  </Loggers>\n"
             + "</Configuration>";
    }

    private static File writeTempConfig(String xml) throws Exception {
        File f = File.createTempFile("log4j2-bench-", ".xml");
        f.deleteOnExit();
        Files.writeString(f.toPath(), xml);
        return f;
    }

    private static String repeat(char c, int n) {
        StringBuilder sb = new StringBuilder(n);
        for (int i = 0; i < n; i++) sb.append(c);
        return sb.toString();
    }
}
