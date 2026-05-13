package org.apache.logging.benchmark;

import org.openjdk.jmh.annotations.*;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.config.Configurator;
import org.apache.logging.log4j.core.impl.Log4jContextFactory;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.core.NativeLogger;
import java.util.concurrent.TimeUnit;
import java.io.File;

@State(Scope.Benchmark)
@BenchmarkMode(Mode.Throughput)
@OutputTimeUnit(TimeUnit.SECONDS)
@Fork(1)
@Warmup(iterations = 1, time = 2, timeUnit = TimeUnit.SECONDS)
@Measurement(iterations = 2, time = 5, timeUnit = TimeUnit.SECONDS)
public class LoggingBenchmark {

    private Logger standardLogger;
    private Logger rlogSpiLogger;
    private LoggerContext ctx;

    @Setup(Level.Trial)
    public void setup() {
        System.setProperty("log4j2.contextSelector", "org.apache.logging.log4j.core.async.AsyncLoggerContextSelector");

        // RlogContextFactory is registered with higher priority via the service loader,
        // so Configurator.initialize() would return null. Directly instantiate
        // Log4jContextFactory to get a real async Log4j2 context for the baseline.
        File configFile = new File("src/jmh/resources/log4j2-async.xml");
        Log4jContextFactory factory = new Log4jContextFactory();
        ctx = (LoggerContext) factory.getContext(
            LoggingBenchmark.class.getName(), null, null, false, configFile.toURI(), "BenchmarkContext");
        standardLogger = ctx.getLogger(LoggingBenchmark.class.getName());

        // Full SPI path: LogManager → RlogContextFactory → RlogLogger → NativeLogger
        rlogSpiLogger = LogManager.getLogger(LoggingBenchmark.class);
        NativeLogger.log(3, "Benchmarking starting...");
    }

    @TearDown(Level.Trial)
    public void tearDown() {
        if (ctx != null) {
            Configurator.shutdown(ctx);
        }
    }

    @Benchmark
    public void benchmarkStandardLog4j2() {
        standardLogger.info("This is a standard Log4j2 Async log message");
    }

    @Benchmark
    public void benchmarkRlog4jFFM() {
        NativeLogger.log(3, "This is an Rlog4 FFM log message");
    }

    @Benchmark
    public void benchmarkRlog4jSPI() {
        rlogSpiLogger.info("This is an Rlog4 SPI log message — full Log4j2 path");
    }
}
