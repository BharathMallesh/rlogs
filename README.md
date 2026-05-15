<div align="center">
  <h1>rlog4j (rlogs)</h1>
  <p><b>A Blazing Fast, Memory-Safe Logging Framework for Java — Powered by Rust</b></p>
</div>

`rlogs` is a drop-in replacement for Log4j 2 that offloads log formatting, async queuing, and file I/O to a native Rust engine. The Java main thread is never blocked by disk I/O, and the Rust background thread never triggers the JVM garbage collector.

> **Full configuration reference:** [CONFIGURATION.md](CONFIGURATION.md)

---

## Features

- **Universal Java support** — JNI bridge for Java 8–21, FFM bridge for Java 22+. The same JAR selects the right path automatically via Multi-Release JAR (MRJAR).
- **Rust-powered engine** — `crossbeam` lock-free channel + single background thread. No GC allocation on the hot path.
- **Drop-in Log4j2 replacement** — implements the Log4j 2 SPI. Keep `LogManager.getLogger()` and `ThreadContext`; no code changes required.
- **Cross-platform fat JAR** — native libraries for 7 platform/arch combinations bundled in one 2 MB JAR. Extracted to a temp dir at startup; no install step.
- **Structured output** — JSON, XML, Logfmt, or plain text. Format selected via `log4j2.xml` and toggled at runtime via env vars.
- **HMAC-SHA256 audit signing** — tamper-evident `_seq` + `_sig` fields on every record. Zero cost when disabled.
- **Rolling files with retention** — size-based roll, optional gzip compression, age-based deletion or archiving.
- **All features individually configurable** — every feature (console, JSON, nodeId, signing, retention, gzip) has its own env-var toggle. One JAR runs in dev, staging, and production without rebuilding.

---

## Supported Platforms

The `rlog4j-all-platforms.jar` bundles native libraries for:

| OS | Architecture |
|---|---|
| macOS | Apple Silicon (`aarch64`), Intel (`x86_64`) |
| Linux | x86_64, x86 (32-bit), aarch64 (Graviton / Pi 4), armv7 |
| Windows | x86_64 |

---

## Quick Start

### 1. Add the dependency

**Maven** (local file repo — included in rlog-demo-app):
```xml
<dependency>
    <groupId>org.apache.logging.log4j</groupId>
    <artifactId>rlog4j-all-platforms</artifactId>
    <version>0.1.0-SNAPSHOT</version>
</dependency>
```

**Gradle:**
```groovy
implementation 'org.apache.logging.log4j:rlog4j-all-platforms:0.1.0-SNAPSHOT'
```

**JitPack (Maven):**
```xml
<repository>
    <id>jitpack.io</id>
    <url>https://jitpack.io</url>
</repository>

<dependency>
    <groupId>com.github.BharathMallesh</groupId>
    <artifactId>rlogs</artifactId>
    <version>main-SNAPSHOT</version>
</dependency>
```

### 2. Add `log4j2.xml` to your classpath

Place the file at `src/main/resources/log4j2.xml`. The minimal all-features-configurable template:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
  <Appenders>
    <Console enabled="${env:RLOG_CONSOLE:-true}">
      <JsonLayout enabled="${env:RLOG_JSON:-true}"/>
    </Console>
    <RollingFile
        fileName="${env:LOG_DIR:-logs}/${env:LOG_FILE:-app.log}"
        filePattern="${env:LOG_DIR:-logs}/app-%d{yyyy-MM-dd-HH}-%i.log">
      <JsonLayout enabled="${env:RLOG_JSON:-true}"/>
      <LogSigning  enabled="${env:RLOG_SIGNING:-false}" keyEnv="LOG_HMAC_SECRET"/>
      <NodeId      enabled="${env:RLOG_NODE_ID:-true}"  env="NODE_ID" default="node0"/>
      <Policies>
        <SizeBasedTriggeringPolicy size="${env:LOG_MAX_SIZE:-10 MB}"/>
      </Policies>
      <DefaultRolloverStrategy max="10"/>
      <RetentionPolicy
          enabled="${env:RLOG_RETENTION:-true}"
          retentionDays="${env:RLOG_RETENTION_DAYS:-2555}"
          archivePath="${env:LOG_DIR:-logs}/archive"
          compress="${env:RLOG_GZIP:-true}"/>
    </RollingFile>
  </Appenders>
  <Loggers>
    <Root level="${env:LOG_LEVEL:-INFO}">
      <AppenderRef ref="Console"/>
      <AppenderRef ref="RollingFile"/>
    </Root>
  </Loggers>
</Configuration>
```

### 3. Write logs with the standard Log4j 2 API

```java
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.ThreadContext;

public class App {
    private static final Logger log = LogManager.getLogger(App.class);

    public static void main(String[] args) {
        ThreadContext.put("userId", "u-1234");
        log.info("order created");
        ThreadContext.clearAll();
    }
}
```

### 4. Run

**Java 8–21 (JNI — no extra flags needed):**
```bash
java -jar your-app.jar
```

**Java 22+ (FFM — native access flag required):**
```bash
java --enable-native-access=ALL-UNNAMED -jar your-app.jar
```

---

## Feature toggle cheatsheet

| Env var | Default | Effect |
|---|---|---|
| `RLOG_CONSOLE` | `true` | Set `false` to suppress stdout |
| `RLOG_JSON` | `true` | Set `false` for plain-text format |
| `RLOG_NODE_ID` | `true` | Set `false` to remove `nodeId` field |
| `RLOG_SIGNING` | `false` | Set `true` to add HMAC `_sig` field |
| `RLOG_RETENTION` | `true` | Set `false` to skip age-based sweep |
| `RLOG_GZIP` | `true` | Set `false` to skip gzip on roll |
| `RLOG_RETENTION_DAYS` | `2555` | Retention window in days (~7 yr) |
| `LOG_DIR` | `logs` | Log file directory |
| `LOG_FILE` | `app.log` | Active log file name |
| `LOG_MAX_SIZE` | `10 MB` | Roll-over size |
| `LOG_LEVEL` | `INFO` | Root log level |
| `NODE_ID` | `node0` | Node identifier in JSON records |
| `LOG_HMAC_SECRET` | — | Signing key (when `RLOG_SIGNING=true`) |

See [CONFIGURATION.md](CONFIGURATION.md) for the full XML element and attribute reference, signing key formats, output format examples, and deployment recipes.

---

## Repository structure

```
rlog-core/       Rust native engine (cdylib)
rlog4j-api/      Java Log4j2 SPI provider + JNI/FFM bridges + MRJAR build
rlog-benchmark/  JMH benchmark suite
CONFIGURATION.md Full configuration reference
```

---

## Building from source

**1. Build the Rust engine**
```bash
cd rlog-core
cargo build --release
```

**2. Copy native library to resources**
```bash
# macOS Apple Silicon example:
cp rlog-core/target/release/librlog_core.dylib \
   rlog4j-api/src/main/resources/META-INF/native/macos/aarch64/librlog_core.dylib
```

**3. Build the Java framework (requires JDK 22+ for MRJAR compilation)**
```bash
JAVA_HOME=/path/to/jdk-22-or-later ./gradlew :rlog4j-api:jar
```

**4. Publish to local Maven**
```bash
JAVA_HOME=/path/to/jdk-22-or-later ./gradlew publishToMavenLocal
```

---

## Using as a dependency (JitPack)

**Gradle:**
```groovy
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

---

## License

Open source. Use freely in enterprise applications.
