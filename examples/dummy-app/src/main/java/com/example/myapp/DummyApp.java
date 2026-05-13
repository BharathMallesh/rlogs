package com.example.myapp;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;
import org.apache.logging.log4j.Marker;
import org.apache.logging.log4j.MarkerManager;
import org.apache.logging.log4j.ThreadContext;

/**
 * Minimal example: drop-in Log4j2 API usage, but the engine is Rust via JNI/FFM.
 *
 * Run with:
 *   mvn exec:exec
 *
 * The classpath is wired up from src/main/resources/log4j2.xml.
 */
public class DummyApp {

    private static final Logger log = LogManager.getLogger(DummyApp.class);

    public static void main(String[] args) throws Exception {
        log.info("dummy-app starting up");

        // ── Plain logging at every level ────────────────────────────────
        log.trace("a TRACE line — usually filtered out");
        log.debug("a DEBUG line");
        log.info("an INFO line");
        log.warn("a WARN line");
        log.error("an ERROR line");

        // ── MDC (mapped diagnostic context) ─────────────────────────────
        ThreadContext.put("requestId", "REQ-12345");
        ThreadContext.put("userId", "alice");
        log.info("processing request — see %X in the pattern");
        ThreadContext.clearAll();

        // ── NDC (nested diagnostic context) ─────────────────────────────
        ThreadContext.push("checkout");
        ThreadContext.push("payment");
        log.info("inside the nested NDC stack — see %x in the pattern");
        ThreadContext.clearStack();

        // ── Markers ─────────────────────────────────────────────────────
        Marker security = MarkerManager.getMarker("SECURITY");
        log.warn(security, "auth attempt from 192.168.1.42");

        // ── Exceptions with full stack-trace via %ex ───────────────────
        try {
            simulateBusinessFailure();
        } catch (Exception e) {
            log.error("business operation failed", e);
        }

        // ── Parameterised messages (Log4j2 lazy formatting) ─────────────
        for (int i = 1; i <= 3; i++) {
            log.info("user {} placed order #{}", "alice", 10000 + i);
        }

        // Give the async Rust thread time to flush before we exit
        log.info("dummy-app shutting down");
        Thread.sleep(500);
    }

    private static void simulateBusinessFailure() {
        try {
            throw new IllegalStateException("connection pool exhausted");
        } catch (IllegalStateException root) {
            throw new RuntimeException("checkout failed", root);
        }
    }
}
