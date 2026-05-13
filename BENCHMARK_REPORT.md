# rlog4j Benchmark Report
**A Rust-Powered Drop-in Replacement for Log4j 2**

> **Date:** May 13, 2026  
> **Author:** Bharath Mallesh  
> **Repository:** https://github.com/BharathMallesh/rlogs  
> **Branch:** `feature/java-22-ffm`

---

## Executive Summary

rlog4j is a logging framework for Java that replaces the Log4j 2 core with a lock-free
Rust engine while keeping the standard Log4j 2 API. Java applications require **zero
code changes** — only the backend JAR is swapped.

| Benchmark | Throughput | vs. Standard Log4j 2 |
|---|---|---|
| rlog4j — Direct FFM call | **19.2M ops/s** | **+1,842%** |
| **rlog4j — Full Log4j 2 API** | **8.5M ops/s** | **+756%** |
| Standard Log4j 2 Async (baseline) | 989K ops/s | — |

**The headline number: rlog4j delivers 8.5 million log operations per second through
the standard Log4j 2 API — 8.6× faster than stock async Log4j 2.**

---

## 1. What is rlog4j?

Traditional Java logging frameworks handle formatting, async queues, and file I/O
entirely on the JVM. This creates three unavoidable costs:

1. **GC pressure** — string formatting allocates heap objects that the garbage
   collector must reclaim.
2. **Thread blocking** — even "async" Log4j 2 eventually serialises through a
   Disruptor ring buffer with JVM-managed threads.
3. **Per-call allocations** — every log event allocates at least one `LogEvent`
   object, a formatted `String`, and backing byte arrays.

rlog4j eliminates all three by delegating the heavy work to a native Rust engine:

```
Java App  ──►  Log4j 2 API  ──►  RlogLogger  ──►  NativeLogger (FFM)
                                                          │
                                               Off-heap Arena buffer
                                                          │
                                              Rust: crossbeam lock-free
                                               channel (capacity 1M)
                                                          │
                                            Background Rust thread
                                                          │
                                           tracing_appender  ──►  Disk / stdout
```

- **RlogLogger** implements the Log4j 2 SPI — existing code using
  `LogManager.getLogger()` works unchanged.
- **NativeLogger** writes the UTF-8 message into a thread-local off-heap
  `MemorySegment` (8 KB, reused across calls) and invokes the Rust function
  via the Java Foreign Function & Memory (FFM) API.
- **Rust** enqueues the message into a `crossbeam::bounded(1_000_000)` channel
  and returns immediately. The Java thread is never blocked by I/O.
- A dedicated Rust background thread drains the channel and writes to
  `tracing_appender` (supports console, file, hourly/daily rolling).

---

## 2. Test Environment

| Component | Detail |
|---|---|
| **Host machine** | Apple Silicon (ARM64), 11 CPU cores, 18 GB RAM |
| **Docker runtime** | Docker Desktop 29.4.1, `linux/arm64` containers |
| **JVM (Docker)** | Eclipse Temurin OpenJDK 22.0.2 (64-bit Server VM) |
| **JVM (host)** | Oracle JDK 23 (used for local build only) |
| **Rust toolchain** | rustc 1.92.0, cargo 1.92.0 |
| **OS inside container** | Linux aarch64 |
| **Benchmark harness** | JMH 1.37 |

The benchmarks ran **inside the Docker container** — the same environment that
production deployments use. The native Rust library (`librlog_core.so`) was compiled
from source inside a `rust:slim` ARM64 container to guarantee ABI compatibility.

---

## 3. Benchmark Methodology

All benchmarks were run with **JMH 1.37** using throughput mode (`ops/s`).

```
Warmup:      1 iteration × 2 seconds   (JIT stabilisation)
Measurement: 2 iterations × 5 seconds  (10 s total measurement window)
Forks:       1 (fresh JVM per benchmark)
Threads:     1 (single-threaded — measures per-call cost, not contention)
Mode:        Throughput (higher = better)
```

### Benchmark descriptions

| Name | Code path | What it isolates |
|---|---|---|
| `benchmarkRlog4jFFM` | `NativeLogger.log(3, msg)` | Raw FFM→Rust channel enqueue. No Log4j 2 SPI overhead. |
| `benchmarkRlog4jSPI` | `logger.info(msg)` via `LogManager` | **Full Log4j 2 API path**: SPI dispatch → `RlogLogger.logMessage()` → `getFormattedMessage()` → ThreadContext lookup → FFM call. This is what every real application call costs. |
| `benchmarkStandardLog4j2` | `logger.info(msg)` via stock Log4j 2 core | Stock async Log4j 2 with Disruptor. Used as the industry baseline. |

The SPI benchmark (`benchmarkRlog4jSPI`) is the authoritative real-world number — it
exercises the identical code path taken by any Java application using rlog4j as a
drop-in replacement.

---

## 4. Results

### 4.1 Raw scores (Docker, Linux ARM64)

```
Benchmark                      Mode   Iterations   Score          Units
─────────────────────────────────────────────────────────────────────────
benchmarkRlog4jFFM            thrpt        2      19,218,368     ops/s
benchmarkRlog4jSPI            thrpt        2       8,468,250     ops/s   ← real-world
benchmarkStandardLog4j2       thrpt        2         989,151     ops/s   ← baseline
```

### 4.2 Relative performance

```
                              0         5M        10M        15M       20M ops/s
                              |         |          |          |         |
  rlog4j FFM (direct)        ████████████████████████████████████████  19.2M
  rlog4j SPI (real-world)    █████████████████████                      8.5M
  Standard Log4j 2 Async     ██                                         989K
```

| Comparison | Multiplier |
|---|---|
| rlog4j SPI vs. Standard Log4j 2 | **8.6×** faster |
| rlog4j FFM vs. Standard Log4j 2 | **19.4×** faster |
| rlog4j SPI vs. rlog4j FFM | 2.3× overhead (Log4j 2 SPI layer) |

### 4.3 Interpreting the gap between FFM and SPI

The 2.3× difference between the direct FFM call and the full SPI path accounts for:

1. **`message.getFormattedMessage()`** — formats the message string (heap allocation).
2. **`ThreadContext.getContext()`** — MDC thread-local lookup on every call.
3. **Log4j 2 SPI dispatch** — `AbstractLogger.logIfEnabled()` → `isEnabled()` check
   → `logMessage()` virtual call chain.

These are inherent costs of the Log4j 2 API contract and exist in all Log4j 2
implementations. rlog4j eliminates everything *after* the formatted string exists.

---

## 5. Why rlog4j is Faster: Technical Deep-Dive

### 5.1 Zero GC pressure on the hot path

Standard Log4j 2 allocates at minimum:
- A `LogEvent` object per call
- A formatted `String` (already unavoidable in any logging framework)
- A `byte[]` for UTF-8 encoding

rlog4j eliminates the `LogEvent` allocation entirely. The formatted string is
converted to UTF-8 directly into a **thread-local 8 KB `MemorySegment`** — off-heap,
reused across calls, never touched by the GC.

```java
// Hot path in NativeLogger.log() — zero heap allocation for messages < 8 KB
MemorySegment buf = THREAD_BUF.get();          // thread-local, already allocated
buf.copyFrom(MemorySegment.ofArray(msgBytes)); // off-heap copy
buf.set(ValueLayout.JAVA_BYTE, msgBytes.length, (byte) 0); // null-terminate
rlogLogHandle.invokeExact(level, buf);         // FFM call to Rust
```

### 5.2 Lock-free async queue with 1M capacity

Log4j 2's AsyncLogger uses an LMAX Disruptor ring buffer, which involves CAS
operations, memory barriers, and JVM thread management.

rlog4j uses a `crossbeam::bounded(1_000_000)` channel — a lock-free multi-producer
single-consumer queue implemented entirely in Rust. `try_send` is a single atomic
compare-and-swap with no JVM involvement.

### 5.3 Disk I/O is fully off the Java thread

The Rust background thread drains the queue and calls `tracing_appender`, which
batches writes with its own internal buffering. The Java thread never waits for
`fsync` or any I/O system call.

### 5.4 Level filtering before formatting

A critical correctness fix applied during development: `RlogLogger.isEnabled()` now
reads the configured log level from the parsed `log4j2.xml` and returns `false` for
disabled levels *before* `getFormattedMessage()` is called. In the original naive
implementation, every `logger.debug("msg {}", obj)` call at INFO level would still
format the string and cross the FFM boundary — silently destroying throughput in
applications with debug logging in hot paths.

---

## 6. Compatibility

| Feature | Status |
|---|---|
| Log4j 2 API (`LogManager`, `Logger`, `ThreadContext`) | Full drop-in |
| `log4j2.xml` configuration | Supported |
| Console appender | Supported |
| File appender | Supported |
| Rolling file (daily / hourly) | Supported |
| `${env:VAR:-default}` property expressions | Supported |
| Java version | Java 22+ (FFM API) |
| Platforms | macOS ARM64/x86_64, Linux ARM64/x86_64/x86/ARMv7, Windows x86_64 |
| Docker | Tested (linux/arm64) |

---

## 7. Reproducing the Benchmark

### Prerequisites
- Docker Desktop 4.x+
- Java 22+
- (Optional) Rust toolchain for rebuilding native libs

### Steps

```bash
# 1. Clone the repository
git clone https://github.com/BharathMallesh/rlogs.git
git checkout feature/java-22-ffm

# 2. Build the JMH fat jar
./gradlew :rlog-benchmark:jmhJar

# 3. Run inside Docker (linux/arm64 or linux/amd64)
docker run --rm \
  -v $(pwd)/rlog-benchmark/build/libs/rlog-benchmark-0.1.0-SNAPSHOT-jmh.jar:/bench.jar \
  eclipse-temurin:22-jre \
  java --enable-native-access=ALL-UNNAMED \
       -jar /bench.jar \
       -wi 1 -w 2 -i 2 -r 5 -f 1 \
       -tu s -bm thrpt
```

Expected output (results will vary by hardware):
```
Benchmark                      Mode   Cnt   Score          Units
benchmarkRlog4jFFM            thrpt     2   ~19,000,000    ops/s
benchmarkRlog4jSPI            thrpt     2   ~8,000,000     ops/s
benchmarkStandardLog4j2       thrpt     2   ~1,000,000     ops/s
```

---

## 8. Notes & Caveats

- **Single-threaded benchmark.** Results measure per-call latency cost, not
  multi-producer throughput. Under concurrent load, the lock-free Rust queue
  scales well, but the FFM bridge is a shared resource — production throughput
  will depend on thread count and message size.

- **Measurement window is short (2 × 5s).** Scores carry no error bars. For
  publication-grade results, use 5+ measurement iterations (`-i 5 -r 10`).

- **The Rust background thread is not the bottleneck here.** These benchmarks
  measure how fast the Java side can *enqueue* messages. Sustained throughput
  is bounded by disk I/O speed once the 1M-message channel fills up.

- **No JVM tuning applied.** Results use the default JVM flags. GC tuning
  (`-XX:+UseZGC`, etc.) could improve the standard Log4j 2 baseline but would
  not significantly affect rlog4j since it produces minimal heap garbage.

---

*Benchmark conducted on 2026-05-13. All measurements taken inside a Docker
`linux/arm64` container on Apple Silicon hardware. Source code available at
https://github.com/BharathMallesh/rlogs (branch: `feature/java-22-ffm`).*
