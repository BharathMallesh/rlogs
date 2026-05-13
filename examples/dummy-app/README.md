# dummy-app — rlog4 via GitHub (JitPack)

A minimal Maven project that uses [rlog4](https://github.com/BharathMallesh/rlogs)
as a logging library. The dependency is pulled from GitHub via JitPack — no
local build of rlog4 needed.

## Prerequisites

- **JDK 22 or newer** (FFM requires Java 22+)
- **Maven 3.8+**
- Internet access on first build (so JitPack can fetch the artifact)

Check with:

```bash
java -version    # must be 22 or newer
mvn -version
```

## Run it

```bash
cd examples/dummy-app
mvn compile exec:exec
```

(`compile` is needed because `exec:exec` does not auto-trigger it.)

The first build can take 1–3 minutes because JitPack has to build rlog4 from
the GitHub source on its servers. Subsequent runs are instant — the artifact
is cached in `~/.m2/repository/com/github/BharathMallesh/rlogs/`.

## What you should see

**On the console** (formatted by `%d{HH:mm:ss.SSS} [%t] %-5level [%marker] %logger{36} %X %x - %msg%n%ex`):

```
14:30:01.123 [main] INFO  [] com.example.myapp.DummyApp  - dummy-app starting up
14:30:01.124 [main] DEBUG [] com.example.myapp.DummyApp  - a DEBUG line
14:30:01.124 [main] INFO  [] com.example.myapp.DummyApp  - an INFO line
14:30:01.124 [main] WARN  [] com.example.myapp.DummyApp  - a WARN line
14:30:01.124 [main] ERROR [] com.example.myapp.DummyApp  - an ERROR line
14:30:01.124 [main] INFO  [] com.example.myapp.DummyApp {requestId=REQ-12345, userId=alice} - processing request — see %X in the pattern
14:30:01.124 [main] INFO  [] com.example.myapp.DummyApp  checkout payment - inside the nested NDC stack — see %x in the pattern
14:30:01.124 [main] WARN  [SECURITY] com.example.myapp.DummyApp  - auth attempt from 192.168.1.42
14:30:01.125 [main] ERROR [] com.example.myapp.DummyApp  - business operation failed
java.lang.RuntimeException: checkout failed
    at com.example.myapp.DummyApp.simulateBusinessFailure(DummyApp.java:64)
    ...
Caused by: java.lang.IllegalStateException: connection pool exhausted
    ...
```

**On disk:** `logs/dummy-app.log` will contain the same lines with the
`yyyy-MM-dd HH:mm:ss.SSS` pattern.

## Build a standalone jar

```bash
mvn package
java --enable-native-access=ALL-UNNAMED -jar target/dummy-app-1.0.0.jar
```

The shaded jar includes rlog4 and its native libraries for all platforms
(macOS, Linux, Windows; aarch64 + x86_64), so the resulting jar runs anywhere
with a JDK 22+ installed.

## How the pieces connect

```
DummyApp.java
    │
    │  log.info(…)
    ▼
Log4j2 LogManager (from log4j-api 2.20.0 — transitive)
    │
    │  service-loader finds RlogProvider
    ▼
RlogLogger (in rlog4)
    │
    │  JNI / FFM downcall
    ▼
librlog_core.dylib  ← extracted from META-INF/native/ at startup
    │
    │  formats the line, writes to file
    ▼
logs/dummy-app.log
```

## Switching versions

The `<version>` in `pom.xml` controls which build of rlog4 you get from JitPack.

| You want… | `<version>` |
|---|---|
| Latest commit on `feature/ffm-bridge` branch | `feature~ffm-bridge-SNAPSHOT` |
| Latest commit on `main` branch | `main-SNAPSHOT` |
| Specific commit | first 10 chars of the commit SHA, e.g. `9ef2deec1a` |
| Tagged release | the tag name, e.g. `v0.1.0` |

JitPack converts `/` to `~` in branch names — that's why it's `feature~ffm-bridge`,
not `feature/ffm-bridge`.

## Switching to a local install (offline / no JitPack)

If you'd rather not depend on JitPack, build rlog4 yourself and install it
into your local Maven repository:

```bash
cd /path/to/rlog4
./gradlew :rlog4j-api:publishToMavenLocal
```

Then edit `pom.xml`:

1. Delete the `<repositories>` block (no JitPack needed for local installs).
2. Change the `<dependency>` to:
   ```xml
   <dependency>
       <groupId>org.apache.logging.log4j</groupId>
       <artifactId>rlog4j-all-platforms</artifactId>
       <version>0.1.0-SNAPSHOT</version>
   </dependency>
   ```
