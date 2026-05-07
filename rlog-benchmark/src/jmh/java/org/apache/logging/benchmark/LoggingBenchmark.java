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

    @Benchmark
    public void benchmarkRlog4jJNI() {
        NativeLogger.log(3, "This is an Rlog4 JNI log message");
    }
}
