<div align="center">
  <h1>🚀 Rlog4 (rlogs)</h1>
  <p><b>A Blazing Fast, Memory-Safe, Zero-Copy Logging Framework for Modern Java</b></p>
</div>

`rlogs` is a next-generation logging framework built as a **drop-in replacement for Log4j 2**. By offloading log formatting, asynchronous queues, and file I/O to a lock-free native Rust engine, `rlogs` completely eliminates Garbage Collection (GC) pressure and JVM thread blocking, allowing your Java application to achieve unprecedented throughput.

## ✨ Features
- ⚡ **Zero-Copy FFM Bridge**: Built on the Java 22 Foreign Function & Memory (FFM) API. Log messages cross the Java/Rust boundary via off-heap memory segments, bypassing the JVM Garbage Collector entirely.
- 🦀 **Rust-Powered Engine**: Built with `crossbeam` lock-free queues and the `tracing` ecosystem. The Java main thread is never blocked by disk I/O.
- 🔄 **Drop-in Log4j2 Replacement**: Implements the Log4j 2 API Provider interface. Just replace your Log4j `core` JAR with `rlog4j`, keep your existing `LogManager.getLogger()` code, and you're done.
- 📦 **Zero-Config Cross-Platform**: A single, self-extracting 2MB JAR that runs natively on 7 different hardware architectures without requiring end-users to install anything.
- 📝 **Rolling Files Built-In**: Native time-based (daily/hourly) rolling file support through XML configuration parity.
- 📊 **Blazing Fast**: Achieves over **11.3 Million ops/sec** in JMH benchmarks—significantly outperforming traditional logging frameworks.

---

## 🌍 Supported Platforms
The `rlog4j-all-platforms.jar` comes pre-bundled with native shared libraries for:
- 🍎 **macOS**: Apple Silicon (`aarch64`) & Intel (`x86_64`)
- 🐧 **Linux**: Intel/AMD (`x86_64`), 32-bit (`x86`), ARM64 (AWS Graviton/Raspberry Pi 4), and ARM32 (`armv7`)
- 🪟 **Windows**: Intel/AMD (`x86_64`)

---

## 🚀 How to Use (Integration Guide)

### 1. Prerequisites
- **Java 22+** (Required for the stable FFM API).

### 2. Installation
Add the following JARs to your application's classpath:
1. `log4j-api-2.20.0.jar` (The official Log4j 2 API)
2. `rlog4j-all-platforms.jar` (This framework)

### 3. Configuration
Place a standard `log4j2.xml` file in your `src/main/resources` (or directly on the classpath). 
Example:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
    <Appenders>
        <Console name="Console" target="SYSTEM_OUT">
            <PatternLayout pattern="%d{HH:mm:ss.SSS} [%t] %-5level %logger{36} - %msg%n"/>
        </Console>
        
        <!-- Native Time-Based Rolling File -->
        <RollingFile name="File" fileName="logs/app.log" filePattern="logs/app-%d{yyyy-MM-dd}.log">
            <PatternLayout pattern="%d{yyyy-MM-dd HH:mm:ss} %-5p %c{1}:%L - %m%n" />
            <Policies>
                <TimeBasedTriggeringPolicy />
            </Policies>
        </RollingFile>
    </Appenders>
    <Loggers>
        <Root level="info">
            <AppenderRef ref="Console"/>
            <AppenderRef ref="File"/>
        </Root>
    </Loggers>
</Configuration>
```

### 4. Running Your Application
Because `rlogs` uses the Foreign Function & Memory API to execute native code, you **must** grant it native access flags when launching the JVM:

```bash
java --enable-native-access=ALL-UNNAMED -cp "your-app.jar:rlog4j-all-platforms.jar:log4j-api-2.20.0.jar" com.your.App
```

### 5. Code Example
No custom imports required! Just use standard Log4j:
```java
import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.ThreadContext;

public class App {
    private static final Logger logger = LogManager.getLogger(App.class);

    public static void main(String[] args) {
        ThreadContext.put("userId", "12345");
        logger.info("Hello from a lock-free native Rust engine!");
    }
}
```

---

## 🛠️ Contributing to Rlog4
We welcome contributions! If you want to improve the Java bridge, optimize the Rust engine, or add new Log4j features, here is how you can get started.

### Repository Structure
- `/rlog-core/` - The Rust backend native library.
- `/rlog4j-api/` - The Java Log4j 2 SPI provider and FFM bridge.
- `/rlog-benchmark/` - JMH benchmarking suite.

### Developer Setup
1. Install [Java 22](https://jdk.java.net/22/).
2. Install [Rust](https://rustup.rs/).
3. *(Optional)* Install [Docker](https://www.docker.com/) if you want to cross-compile for Linux/Windows from a Mac.

### Building from Source
**1. Build the Rust Engine (Native)**
```bash
cd rlog-core
cargo build --release
```

**2. Copy the Native Library**
Copy the generated `.so` / `.dylib` / `.dll` from `rlog-core/target/release/` into the correct OS/Architecture folder in `rlog4j-api/src/main/resources/META-INF/native/`.

**3. Compile the Java Framework**
*(We currently use raw `javac` and `jar` for building the final artifact).*
```bash
export JAVA_HOME=/path/to/jdk-22
export PATH=$JAVA_HOME/bin:$PATH

javac -d out/classes -cp libs/log4j-api-2.20.0.jar $(find rlog4j-api/src/main/java -name "*.java")
# Copy resources...
jar --create --file rlog4j-all-platforms.jar -C out/classes .
```

### Architecture Notes for Contributors
- **FFM Over JNI**: We use FFM `MemorySegment.ofAddress()` to pass Java memory pointers directly to C ABI compatible `extern "C"` functions in Rust.
- **Async by Default**: The `rlog_log` Rust function serializes the message into a lock-free `crossbeam::channel` and immediately returns to Java. The actual disk I/O happens on a dedicated background thread pool managed by Rust `tracing`.
- **String Handling**: Java strings are UTF-16. We write them natively into an off-heap Arena buffer as UTF-8 bytes to prevent the JVM from allocating byte arrays during garbage collection.

## 📦 Using as a Dependency (Maven/Gradle)

You can include `rlogs` in your other projects as a standard dependency using two methods:

### Method A: Using JitPack (Cloud)
The easiest way to use `rlogs` in any project without manual installation.

#### Gradle
1. Add JitPack to your `repositories` block:
```gradle
repositories {
    maven { url 'https://jitpack.io' }
}
```
2. Add the dependency:
```gradle
dependencies {
    implementation 'com.github.BharathMallesh:rlogs:main-SNAPSHOT'
}
```

#### Maven
1. Add the JitPack repository:
```xml
<repositories>
    <repository>
        <id>jitpack.io</id>
        <url>https://jitpack.io</url>
    </repository>
</repositories>
```
2. Add the dependency:
```xml
<dependency>
    <groupId>com.github.BharathMallesh</groupId>
    <artifactId>rlogs</artifactId>
    <version>main-SNAPSHOT</version>
</dependency>
```

---

### Method B: Local Maven Repository
If you want to install it on your local machine for use in other local projects.

1. Clone the repository.
2. Run the following command to install it to your `~/.m2/repository`:
```bash
./gradlew publishToMavenLocal
```
3. In your other project, add `mavenLocal()` to your repositories and include:
```gradle
implementation 'org.apache.logging.log4j:rlog4j-all-platforms:0.1.0-SNAPSHOT'
```

---

## 📄 License
This project is open-source. Feel free to use it in your enterprise applications to crush your GC latency!
