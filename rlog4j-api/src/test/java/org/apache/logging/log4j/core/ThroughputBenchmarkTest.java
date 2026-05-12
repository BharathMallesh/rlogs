package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.junit.jupiter.api.Test;

import java.io.BufferedWriter;
import java.io.File;
import java.io.FileWriter;
import java.text.SimpleDateFormat;
import java.util.Date;

/**
 * Throughput benchmark: rlog4 vs hand-rolled Java BufferedWriter.
 *
 * Both write the same formatted line to the same file pattern.
 * Single-threaded, steady-state, 500_000 events with 50_000 warmup.
 *
 * The Java baseline approximates "what Log4j2 sync to file is doing" —
 * synchronous format + write — without the actual log4j-core machinery
 * (which conflicts with rlog4j-api on this classpath).
 *
 * Results are printed; nothing is asserted (so this never fails CI).
 */
public class ThroughputBenchmarkTest {

    private static final int WARMUP = 50_000;
    private static final int EVENTS = 500_000;

    @Test
    public void benchmarkThroughput() throws Exception {
        System.out.println("\n=== Throughput benchmark: rlog4 vs Java BufferedWriter baseline ===\n");
        System.out.println("warmup = " + WARMUP + " events, measured = " + EVENTS + " events");
        System.out.println("Single-threaded, file appender, pattern with timestamp + level + logger + msg\n");

        // ── 1. rlog4 full path (with caller location capture) ──────────────
        File rlog4File = File.createTempFile("rlog4-bench-", ".log");
        rlog4File.deleteOnExit();
        long rlog4Result = benchmarkRlog4(rlog4File);

        // ── 1b. rlog4 direct JNI (bypass RlogLogger overhead) ───────────────
        File rlog4DirectFile = File.createTempFile("rlog4-direct-", ".log");
        rlog4DirectFile.deleteOnExit();
        long rlog4DirectResult = benchmarkRlog4Direct(rlog4DirectFile);

        // Reset to default config so the comparison is fair
        Configurator.reconfigure(defaultXml());
        Thread.sleep(300);

        // ── 2. Java BufferedWriter baseline ─────────────────────────────────
        File javaFile = File.createTempFile("java-bench-", ".log");
        javaFile.deleteOnExit();
        long javaResult = benchmarkJavaBufferedWriter(javaFile);

        // ── 3. Java synchronous (no buffer) — closest to "no logger framework" ─
        File rawFile = File.createTempFile("raw-bench-", ".log");
        rawFile.deleteOnExit();
        long rawResult = benchmarkJavaRawFile(rawFile);

        // ── Report ──────────────────────────────────────────────────────────
        System.out.println("\n" + repeat('═', 70));
        System.out.println("RESULTS — events per second (higher = better)");
        System.out.println(repeat('═', 70));
        System.out.printf("rlog4 full (LogManager→RlogLogger→JNI) %,12d events/sec  (file: %,d bytes)%n",
                rlog4Result, rlog4File.length());
        System.out.printf("rlog4 direct JNI (NativeLogger.log)     %,12d events/sec  (file: %,d bytes)%n",
                rlog4DirectResult, rlog4DirectFile.length());
        System.out.printf("Java BufferedWriter (8KB buf) %,12d events/sec  (file size: %,d bytes)%n",
                javaResult, javaFile.length());
        System.out.printf("Java FileWriter (no buffer)   %,12d events/sec  (file size: %,d bytes)%n",
                rawResult, rawFile.length());
        System.out.println(repeat('═', 70));

        double ratio = (double) rlog4Result / javaResult;
        if (ratio >= 1.0) {
            System.out.printf("→ rlog4 is %.2fx faster than Java BufferedWriter%n", ratio);
        } else {
            System.out.printf("→ Java BufferedWriter is %.2fx faster than rlog4%n", 1.0 / ratio);
        }
        System.out.println();
        System.out.println("Notes:");
        System.out.println("  • rlog4 path: logger.info() → JNI → Rust pattern format → File::write");
        System.out.println("  • Java path:  manual SimpleDateFormat + String.format + BufferedWriter.write");
        System.out.println("  • Log4j2 sync would be in the same ballpark as the BufferedWriter baseline");
        System.out.println("  • Log4j2 async (Disruptor) typically 3-5x the sync number = 10-15M events/s on this hardware");
        System.out.println();
    }

    // ──────────────────────────────────────────────────────────────────────────

    private long benchmarkRlog4(File logFile) throws Exception {
        // Configure rlog4 to write to our temp file with a standard production pattern
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <File name=\"BenchFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"%d{yyyy-MM-dd HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"BenchFile\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
        Configurator.reconfigure(xml);
        Thread.sleep(300);

        Logger logger = LogManager.getLogger("com.benchmark.RlogService");

        System.out.println("[1/3] rlog4 warmup (" + WARMUP + " events)...");
        for (int i = 0; i < WARMUP; i++) {
            logger.info("Warmup event " + i);
        }
        Thread.sleep(300); // let queue drain

        System.out.println("      rlog4 measuring (" + EVENTS + " events)...");
        long start = System.nanoTime();
        for (int i = 0; i < EVENTS; i++) {
            logger.info("Benchmark event message number " + i);
        }
        long elapsed = System.nanoTime() - start;

        // Wait for the async queue to fully drain so the file size is accurate
        Thread.sleep(1000);

        double seconds = elapsed / 1_000_000_000.0;
        long eventsPerSec = (long) (EVENTS / seconds);
        System.out.printf("      rlog4 done: %,d events in %.3fs → %,d events/sec%n",
                EVENTS, seconds, eventsPerSec);
        return eventsPerSec;
    }

    private long benchmarkRlog4Direct(File logFile) throws Exception {
        // Same pattern as the RlogLogger path, but skip all Java-side work
        String xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <File name=\"BenchFile\" fileName=\"" + logFile.getAbsolutePath() + "\">\n"
                + "      <PatternLayout pattern=\"%d{yyyy-MM-dd HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>\n"
                + "    </File>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"BenchFile\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
        Configurator.reconfigure(xml);
        Thread.sleep(300);

        System.out.println("[1b/4] rlog4 direct JNI warmup...");
        for (int i = 0; i < WARMUP; i++) {
            NativeLogger.log(3, "Warmup event " + i);
        }
        Thread.sleep(300);

        System.out.println("       rlog4 direct JNI measuring...");
        long start = System.nanoTime();
        for (int i = 0; i < EVENTS; i++) {
            NativeLogger.log(3, "Benchmark event message number " + i);
        }
        long elapsed = System.nanoTime() - start;
        Thread.sleep(1000);

        double seconds = elapsed / 1_000_000_000.0;
        long eventsPerSec = (long) (EVENTS / seconds);
        System.out.printf("       rlog4 direct JNI done: %,d events in %.3fs → %,d events/sec%n",
                EVENTS, seconds, eventsPerSec);
        return eventsPerSec;
    }

    private long benchmarkJavaBufferedWriter(File logFile) throws Exception {
        SimpleDateFormat fmt = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS");
        String thread = Thread.currentThread().getName();
        String loggerName = "com.benchmark.JavaService";

        try (BufferedWriter w = new BufferedWriter(new FileWriter(logFile))) {
            System.out.println("[2/3] Java BufferedWriter warmup...");
            for (int i = 0; i < WARMUP; i++) {
                String line = fmt.format(new Date()) + " [" + thread + "] INFO  " +
                              loggerName + " - Warmup event " + i + "\n";
                w.write(line);
            }
            w.flush();

            System.out.println("      Java BufferedWriter measuring...");
            long start = System.nanoTime();
            for (int i = 0; i < EVENTS; i++) {
                String line = fmt.format(new Date()) + " [" + thread + "] INFO  " +
                              loggerName + " - Benchmark event message number " + i + "\n";
                w.write(line);
            }
            w.flush();
            long elapsed = System.nanoTime() - start;

            double seconds = elapsed / 1_000_000_000.0;
            long eventsPerSec = (long) (EVENTS / seconds);
            System.out.printf("      Java BufferedWriter done: %,d events in %.3fs → %,d events/sec%n",
                    EVENTS, seconds, eventsPerSec);
            return eventsPerSec;
        }
    }

    private long benchmarkJavaRawFile(File logFile) throws Exception {
        SimpleDateFormat fmt = new SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS");
        String thread = Thread.currentThread().getName();
        String loggerName = "com.benchmark.RawService";

        try (FileWriter w = new FileWriter(logFile)) {
            System.out.println("[3/3] Java FileWriter (unbuffered) warmup...");
            for (int i = 0; i < WARMUP; i++) {
                String line = fmt.format(new Date()) + " [" + thread + "] INFO  " +
                              loggerName + " - Warmup event " + i + "\n";
                w.write(line);
            }

            System.out.println("      Java FileWriter measuring...");
            long start = System.nanoTime();
            for (int i = 0; i < EVENTS; i++) {
                String line = fmt.format(new Date()) + " [" + thread + "] INFO  " +
                              loggerName + " - Benchmark event message number " + i + "\n";
                w.write(line);
            }
            long elapsed = System.nanoTime() - start;

            double seconds = elapsed / 1_000_000_000.0;
            long eventsPerSec = (long) (EVENTS / seconds);
            System.out.printf("      Java FileWriter done: %,d events in %.3fs → %,d events/sec%n",
                    EVENTS, seconds, eventsPerSec);
            return eventsPerSec;
        }
    }

    private static String defaultXml() {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"
                + "<Configuration status=\"WARN\">\n"
                + "  <Appenders>\n"
                + "    <Console name=\"Console\" target=\"SYSTEM_OUT\">\n"
                + "      <PatternLayout pattern=\"%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n\"/>\n"
                + "    </Console>\n"
                + "  </Appenders>\n"
                + "  <Loggers>\n"
                + "    <Root level=\"INFO\"><AppenderRef ref=\"Console\"/></Root>\n"
                + "  </Loggers>\n"
                + "</Configuration>";
    }

    private static String repeat(char c, int n) {
        StringBuilder sb = new StringBuilder(n);
        for (int i = 0; i < n; i++) sb.append(c);
        return sb.toString();
    }
}
