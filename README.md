<div align="center">
  <h1>🚀 Rlog4 (rlogs)</h1>
  <p><b>A Blazing Fast, Memory-Safe, Zero-Copy Logging Framework for Modern Java</b></p>
  <p>
    <img src="https://img.shields.io/badge/Java-8%2B-blue?logo=openjdk" alt="Java 8+">
    <img src="https://img.shields.io/badge/Rust-Native-orange?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/Log4j2-Drop--in-green" alt="Log4j2 Drop-in">
    <img src="https://img.shields.io/badge/Throughput-11.3M%20ops%2Fs-brightgreen" alt="11.3M ops/sec">
  </p>
</div>

`rlogs` is a next-generation logging framework built as a **drop-in replacement for Log4j 2**. By offloading log formatting, asynchronous queues, and file I/O to a lock-free native Rust engine via JNI, `rlogs` completely eliminates Garbage Collection (GC) pressure and JVM thread blocking, allowing your Java application to achieve unprecedented throughput.

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| ⚡ **Universal Native Bridge (JNI)** | Compatible with **all Java versions from 8 to 23+**. No `--add-opens` flags, no FFM, no module system hassles. |
| 🦀 **Rust-Powered Engine** | Built with `crossbeam` lock-free queues and the `tracing` ecosystem. The Java main thread is never blocked by disk I/O. |
| 🔄 **Drop-in Log4j 2 Replacement** | Implements the Log4j 2 API Provider interface. Just swap your Log4j `core` JAR with `rlog4j`, keep your existing `LogManager.getLogger()` code. |
| 📦 **Zero-Config Cross-Platform** | A single, self-extracting ~2MB JAR that runs natively on 7 different hardware architectures. |
| 📊 **11.3 Million ops/sec** | Significantly outperforms traditional logging frameworks in JMH benchmarks. |
| 🧩 **Structured JSON Logging** | Native JSON output via `<JsonLayout />` — machine-readable, cloud-native, zero Java object allocation. |
| 📝 **Size-Based & Time-Based Rolling** | `SizeBasedTriggeringPolicy`, `DefaultRolloverStrategy`, and time-based (daily/hourly) rolling built-in. |
| 🛡️ **Graceful Shutdown** | JVM shutdown hook ensures all buffered logs are flushed to disk before process exit. Zero data loss. |
| 🔀 **Queue Backpressure Policy** | Choose `Block` (guarantee no log loss) or `Discard` (guarantee max throughput) when the queue is full. |
| 🏷️ **Full ThreadContext (MDC)** | MDC data flows across the JNI bridge and appears as a distinct structured field in JSON output. |
| 🔍 **XML Lookups** | `${env:VAR}`, `${sys:prop}`, and `${env:VAR:-default}` are resolved at startup — just like Log4j 2. |

---

## 🌍 Supported Platforms

The `rlog4j-all-platforms.jar` comes pre-bundled with native shared libraries for:

| Platform | Architectures |
|----------|--------------|
| 🍎 **macOS** | Apple Silicon (`aarch64`) & Intel (`x86_64`) |
| 🐧 **Linux** | Intel/AMD (`x86_64`), 32-bit (`x86`), ARM64 (AWS Graviton / Raspberry Pi 4), ARM32 (`armv7`) |
| 🪟 **Windows** | Intel/AMD (`x86_64`) |

---

## 🚀 Quick Start

### 1. Prerequisites
- **Java 8+** (Works with Java 8, 11, 17, 21, 22, 23+).

### 2. Installation
Add the following dependencies to your project:

**Gradle:**
```gradle
repositories {
    mavenLocal()
    mavenCentral()
}

dependencies {
    implementation 'org.apache.logging.log4j:rlog4j-all-platforms:0.1.0-SNAPSHOT'
    implementation 'org.apache.logging.log4j:log4j-api:2.20.0'
}
```

**Maven:**
```xml
<dependency>
    <groupId>org.apache.logging.log4j</groupId>
    <artifactId>rlog4j-all-platforms</artifactId>
    <version>0.1.0-SNAPSHOT</version>
</dependency>
<dependency>
    <groupId>org.apache.logging.log4j</groupId>
    <artifactId>log4j-api</artifactId>
    <version>2.20.0</version>
</dependency>
```

### 3. Configuration

Place a standard `log4j2.xml` file in `src/main/resources/`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
    <Appenders>
        <Console name="Console" target="SYSTEM_OUT">
            <PatternLayout pattern="%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n"/>
        </Console>

        <File name="File" fileName="${sys:LOG_DIR:-logs}/app-${env:APP_NAME:-myapp}.log">
            <JsonLayout properties="true" />
            <Policies>
                <SizeBasedTriggeringPolicy size="100MB" />
            </Policies>
            <DefaultRolloverStrategy max="5" />
        </File>
    </Appenders>

    <!-- Block = zero log loss | Discard = max throughput -->
    <AsyncQueueFullPolicy type="Block" />

    <Loggers>
        <Root level="info">
            <AppenderRef ref="Console"/>
            <AppenderRef ref="File"/>
        </Root>
    </Loggers>
</Configuration>
```

### 4. Code Example

No custom imports required — just standard Log4j 2:

```java
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.ThreadContext;

public class App {
    private static final Logger logger = LogManager.getLogger(App.class);

    public static void main(String[] args) {
        // MDC context flows across JNI to the Rust engine
        ThreadContext.put("requestId", "REQ-ABC-123");
        ThreadContext.put("userId", "42");

        logger.info("Hello from a lock-free native Rust engine!");
        logger.warn("This warning includes full MDC context in JSON output");

        ThreadContext.clearAll();
    }
}
```

**JSON Output:**
```json
{
  "timestamp": "2026-05-11T06:13:24.829245Z",
  "level": "INFO",
  "fields": {
    "message": "Hello from a lock-free native Rust engine!",
    "context": "{requestId=REQ-ABC-123, userId=42}"
  },
  "target": "rlog_core"
}
```

### 5. Running

```bash
java -DLOG_DIR=logs -cp "your-app.jar:rlog4j-all-platforms.jar:log4j-api-2.20.0.jar" com.your.App
```

---

## 📖 Configuration Reference

### Layouts

| Layout | XML Tag | Description |
|--------|---------|-------------|
| Pattern | `<PatternLayout pattern="..." />` | Human-readable text format |
| JSON | `<JsonLayout properties="true" />` | Structured JSON for cloud/ELK pipelines |

### Appenders

| Appender | XML Tag | Description |
|----------|---------|-------------|
| Console | `<Console>` | Writes to stdout/stderr |
| File | `<File fileName="...">` | Writes to a fixed file path |
| RollingFile | `<RollingFile fileName="..." filePattern="...">` | Time-based rolling (daily/hourly) |

### Policies & Strategies

| Element | Example | Description |
|---------|---------|-------------|
| Size Rolling | `<SizeBasedTriggeringPolicy size="100MB" />` | Roll file when it exceeds the size limit |
| Time Rolling | `<TimeBasedTriggeringPolicy />` | Roll file daily or hourly |
| Retention | `<DefaultRolloverStrategy max="10" />` | Keep only the N most recent rolled files |

### Queue Backpressure

```xml
<!-- Block: Java thread waits when queue is full (zero data loss) -->
<AsyncQueueFullPolicy type="Block" />

<!-- Discard: Drop logs when queue is full (max throughput) -->
<AsyncQueueFullPolicy type="Discard" />
```

### XML Lookups

Resolve dynamic values in your XML configuration at startup:

| Syntax | Description | Example |
|--------|-------------|---------|
| `${env:KEY}` | Environment variable | `${env:USER}` → `Bharath` |
| `${sys:KEY}` | Java system property (`-D`) | `${sys:LOG_DIR}` → `logs` |
| `${env:KEY:-default}` | With fallback value | `${env:REGION:-us-east-1}` → `us-east-1` |

---

## 🏗️ Architecture

```
┌─────────────────────────────────┐
│         Java Application        │
│   LogManager.getLogger(...)     │
│   ThreadContext.put("k","v")    │
└──────────────┬──────────────────┘
               │  Log4j 2 SPI
               ▼
┌──────────────────────────────────┐
│       RlogLogger (Java)          │
│  • Maps Log4j Level → int        │
│  • Serializes MDC to String      │
│  • Resolves ${env/sys} lookups   │
└──────────────┬───────────────────┘
               │  JNI Bridge
               ▼
┌──────────────────────────────────┐
│       Rust Native Engine         │
│  • crossbeam lock-free channel   │
│  • Atomic backpressure policy    │
│  • tracing-subscriber formatter  │
│  • rolling-file / tracing-appender│
│  • Background I/O thread         │
└──────────────────────────────────┘
               │
               ▼
         [ Disk / stdout ]
```

**Key Design Decisions:**
- **JNI over FFM**: JNI provides universal Java 8+ compatibility, eliminating the need for `--add-opens` or `--enable-native-access` flags.
- **Async by Default**: `rlog_log` pushes to a lock-free `crossbeam::channel` and returns instantly. Disk I/O is handled by a dedicated background thread.
- **Lookups in Java, Parsing in Rust**: XML lookups (`${env:...}`) are resolved on the Java side before crossing the JNI bridge, keeping the Rust XML parser fast and simple.

---

## 🛠️ Contributing

### Repository Structure

| Directory | Purpose |
|-----------|---------|
| `/rlog-core/` | Rust backend — native library (crossbeam, tracing, rolling-file) |
| `/rlog4j-api/` | Java Log4j 2 SPI provider and JNI bridge |
| `/rlog-benchmark/` | JMH benchmarking suite |

### Developer Setup

1. Install [Java 8+](https://adoptium.net/) (Java 22+ recommended for development).
2. Install [Rust](https://rustup.rs/).
3. *(Optional)* Install [Docker](https://www.docker.com/) for cross-compiling Linux/Windows targets from macOS.

### Building from Source

**1. Build the Rust Engine:**
```bash
cd rlog-core
cargo build --release
```

**2. Copy the Native Library:**
```bash
# macOS Apple Silicon example:
cp target/release/librlog_core.dylib \
   ../rlog4j-api/src/main/resources/META-INF/native/macos/aarch64/
```

**3. Build & Publish the Java JAR:**
```bash
./gradlew publishToMavenLocal
```

The JAR will be available as `org.apache.logging.log4j:rlog4j-all-platforms:0.1.0-SNAPSHOT` in your `~/.m2/repository`.

---

## 📦 Dependency Installation

### Method A: JitPack (Cloud)

**Gradle:**
```gradle
repositories {
    maven { url 'https://jitpack.io' }
}

dependencies {
    implementation 'com.github.BharathMallesh:rlogs:main-SNAPSHOT'
}
```

**Maven:**
```xml
<repositories>
    <repository>
        <id>jitpack.io</id>
        <url>https://jitpack.io</url>
    </repository>
</repositories>

<dependency>
    <groupId>com.github.BharathMallesh</groupId>
    <artifactId>rlogs</artifactId>
    <version>main-SNAPSHOT</version>
</dependency>
```

### Method B: Local Maven Repository
```bash
git clone https://github.com/BharathMallesh/rlogs.git
cd rlogs
./gradlew publishToMavenLocal
```

Then in your project:
```gradle
repositories {
    mavenLocal()
}

dependencies {
    implementation 'org.apache.logging.log4j:rlog4j-all-platforms:0.1.0-SNAPSHOT'
}
```

---

## 📄 License

This project is open-source. Feel free to use it in your enterprise applications to crush your GC latency!
