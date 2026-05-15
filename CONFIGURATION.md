# rlog4j Configuration Reference

rlog4j is configured via a standard `log4j2.xml` file on the classpath.
Every feature can be toggled or tuned without changing code — all XML
attribute values accept the `${env:VAR_NAME:-default}` substitution syntax,
so a single config file works across every environment.

---

## Quick-start template

```xml
<?xml version="1.0" encoding="UTF-8"?>
<Configuration status="WARN">
  <Appenders>

    <!-- Console: structured JSON to stdout -->
    <Console enabled="${env:RLOG_CONSOLE:-true}">
      <JsonLayout enabled="${env:RLOG_JSON:-true}"/>
    </Console>

    <!-- Rolling file appender -->
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

---

## Environment variable reference

### Feature toggles

| Variable | Default | Description |
|---|---|---|
| `RLOG_CONSOLE` | `true` | Enable/disable console (stdout) output |
| `RLOG_JSON` | `true` | Use JSON layout. Set `false` for plain-text (tracing-subscriber format) |
| `RLOG_NODE_ID` | `true` | Include `nodeId` field in every JSON/XML/Logfmt record |
| `RLOG_SIGNING` | `false` | Enable HMAC-SHA256 audit signing (`_seq` + `_sig` fields) |
| `RLOG_RETENTION` | `true` | Enable age-based log retention sweep |
| `RLOG_GZIP` | `true` | Gzip rolled-over log files |

### File and directory tunables

| Variable | Default | Description |
|---|---|---|
| `LOG_DIR` | `logs` | Base directory for log files and archive |
| `LOG_FILE` | `app.log` | Active log file name |
| `LOG_MAX_SIZE` | `10 MB` | Roll-over threshold. Accepts `KB`, `MB`, `GB` |
| `LOG_LEVEL` | `INFO` | Root log level: `TRACE`, `DEBUG`, `INFO`, `WARN`, `ERROR` |

### Node identity

| Variable | Default | Description |
|---|---|---|
| `NODE_ID` | `node0` | Node identifier stamped in the `nodeId` field |

### HMAC signing (requires `RLOG_SIGNING=true`)

| Variable | Default | Description |
|---|---|---|
| `LOG_HMAC_SECRET` | — | Signing key. Prefix with `hex:` for hex-encoded keys, `base64:` for base64, or supply a raw UTF-8 passphrase |

---

## XML element reference

### `<Console enabled="...">`

Controls whether log output is written to stdout.

| Attribute | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | `false` suppresses all console output |

### `<JsonLayout enabled="...">`

Switches the output format between structured JSON and plain text.

| Attribute | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | `false` keeps the appender active but switches to plain text (tracing-subscriber format) |

When JSON is enabled each line is a self-contained JSON object:

```json
{"@timestamp":"2026-05-15T11:50:01.632Z","level":"INFO","nodeId":"payment-svc-3","message":"order created","orderId":"ORD-9182"}
```

When disabled, output is plain text with ANSI colour codes (suitable for human-readable terminal output, not machine parsing).

### `<RollingFile fileName="..." filePattern="...">`

Standard rolling file appender.

| Attribute | Type | Description |
|---|---|---|
| `fileName` | path | Active log file path. Supports `${env:...}` substitution |
| `filePattern` | pattern | Archive file pattern. `%d{...}` triggers time-based rolling; `%i` is the index suffix |

### `<SizeBasedTriggeringPolicy size="...">`

Roll the file when it reaches the given size.

| Attribute | Format | Examples |
|---|---|---|
| `size` | number + unit | `10 MB`, `512 KB`, `1 GB` |

### `<DefaultRolloverStrategy max="...">`

Number of backup files to keep alongside the active log.

| Attribute | Type | Default |
|---|---|---|
| `max` | int | `10` |

### `<RetentionPolicy ...>`

Age-based sweep that deletes (or gzip-archives) log files older than `retentionDays`.

| Attribute | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | `false` disables the retention sweep entirely |
| `retentionDays` | int | `2555` | Files older than this many days are deleted. 2555 ≈ 7 years |
| `archivePath` | path | — | Move expired files here instead of deleting them |
| `compress` | bool | `true` | Gzip files when rolling and before archiving |

### `<NodeId ...>`

Stamps a `nodeId` field into every log record for multi-node correlation.

| Attribute | Type | Description |
|---|---|---|
| `enabled` | bool | `false` removes the field from the output entirely |
| `value` | string | Static node identifier (e.g. `payment-node-1`) |
| `env` | string | Name of an environment variable to read at startup (e.g. `NODE_ID`) |
| `default` | string | Fallback when `env` variable is not set |

Priority: `value` > `env` > `default` > auto-detected hostname.

### `<LogSigning ...>`

Appends an HMAC-SHA256 signature (`_sig`) and monotonic sequence number (`_seq`) to every log record. Used for tamper-evident audit logs.

| Attribute | Type | Description |
|---|---|---|
| `enabled` | bool | `false` skips signing (no performance cost) |
| `keyEnv` | string | Name of the environment variable holding the signing key |
| `keyFile` | path | Path to a file containing the signing key |
| `keyHex` | string | Hex-encoded key inline in the XML |

Key format rules (for all three sources):
- Prefix `hex:` → hex-decoded bytes
- Prefix `base64:` → base64-decoded bytes
- No prefix → raw UTF-8 passphrase (HMAC pads to 64 bytes internally)

Signed JSON output example:

```json
{
  "@timestamp": "2026-05-15T11:50:28.766Z",
  "level": "INFO",
  "nodeId": "payment-svc-3",
  "message": "payment captured",
  "_seq": 42,
  "_sig": "RlZ+X7yhYSO2msmD8tx0gqFRO6Pw6TPIdJkVaflowG0="
}
```

The canonical string that is signed is:
```
LEVEL [nodeId] message mdc-pairs _seq=N
```

Verifying a record in Python:
```python
import hmac, hashlib, base64

key   = b"mysecretkey123"
canon = 'INFO [payment-svc-3] payment captured _seq=42'
sig   = base64.b64encode(
    hmac.new(key, canon.encode(), hashlib.sha256).digest()
).decode()
assert sig == record["_sig"]
```

### `<XmlLayout enabled="...">` / `<LogfmtLayout enabled="...">`

Alternative structured output formats. Same `enabled` attribute as `<JsonLayout>`.

XML output example:
```xml
<event timestamp="2026-05-15T11:50:28.766Z" level="INFO" nodeId="payment-svc-3">
  <message>payment captured</message>
  <context><entry key="orderId" value="ORD-9182"/></context>
</event>
```

Logfmt output example:
```
ts=2026-05-15T11:50:28.766Z level=INFO nodeId=payment-svc-3 msg="payment captured" orderId=ORD-9182
```

---

## Env-var substitution syntax

Any XML attribute value can contain one or more `${env:VAR:-default}` expressions:

```
${env:VAR_NAME}           # required — startup fails if VAR_NAME is unset
${env:VAR_NAME:-fallback} # optional — uses fallback when VAR_NAME is unset
```

Multiple substitutions in one value are resolved left-to-right:

```xml
fileName="${env:LOG_DIR:-logs}/${env:LOG_FILE:-app.log}"
```

---

## Typical deployment configurations

### Development (verbose, no signing, plain text)

```bash
RLOG_JSON=false RLOG_NODE_ID=false LOG_LEVEL=DEBUG java -jar app.jar
```

### Staging (JSON, with nodeId, without signing)

```bash
LOG_LEVEL=INFO NODE_ID=staging-node-1 java -jar app.jar
```

### Production (all features on, HMAC signing, 7-year retention)

```bash
RLOG_SIGNING=true \
LOG_HMAC_SECRET="$(cat /run/secrets/log_hmac_key)" \
NODE_ID="$(hostname)" \
LOG_DIR=/var/log/myapp \
LOG_LEVEL=WARN \
java -jar app.jar
```

### Minimal / high-throughput (no console, no nodeId, no signing, no retention)

```bash
RLOG_CONSOLE=false \
RLOG_NODE_ID=false \
RLOG_SIGNING=false \
RLOG_RETENTION=false \
LOG_LEVEL=WARN \
java -jar app.jar
```

This mode is closest in overhead to a bare file-write logger.

---

## Java version and native bridge

| Java version | Native bridge | Notes |
|---|---|---|
| 8 – 21 | JNI | Automatic — no flags required |
| 22+ | FFM | Add `--enable-native-access=ALL-UNNAMED` to JVM args |

The same JAR detects the JVM version at startup and loads the correct bridge automatically via the Multi-Release JAR mechanism.
