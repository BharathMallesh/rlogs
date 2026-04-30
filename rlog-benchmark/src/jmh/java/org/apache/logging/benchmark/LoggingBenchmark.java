package org.apache.logging.benchmark;

import org.openjdk.jmh.annotations.*;
import org.apache.logging.log4j.core.LoggerContext;
import org.apache.logging.log4j.core.config.Configurator;
import org.apache.logging.log4j.Logger;
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
    private LoggerContext ctx;

    @Setup(Level.Trial)
    public void setup() {
        // Force Log4j2 to use Async Loggers globally
        System.setProperty("log4j2.contextSelector", "org.apache.logging.log4j.core.async.AsyncLoggerContextSelector");
        
        // Initialize standard Log4j 2 core explicitly to bypass RlogProvider if needed
        File configFile = new File("src/jmh/resources/log4j2-async.xml");
        ctx = Configurator.initialize("BenchmarkContext", null, configFile.toURI());
        standardLogger = ctx.getLogger(LoggingBenchmark.class.getName());
        
        // Initialize our Native FFM logger
        // NativeLogger is initialized automatically in its static block
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
}
