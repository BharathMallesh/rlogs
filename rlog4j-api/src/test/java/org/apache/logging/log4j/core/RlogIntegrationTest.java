package org.apache.logging.log4j.core;

import org.apache.logging.log4j.LogManager;
import org.apache.logging.log4j.Logger;

public class RlogIntegrationTest {
    public static void main(String[] args) throws Exception {
        System.out.println("Starting Log4j 2 API Integration Test...");
        
        // This should trigger the ServiceLoader to find RlogProvider
        Logger logger = LogManager.getLogger(RlogIntegrationTest.class);
        
        org.apache.logging.log4j.ThreadContext.put("requestId", "txn-12345");
        org.apache.logging.log4j.ThreadContext.put("userId", "usr-987");
        
        logger.info("This is an INFO message routed through standard LogManager!");
        
        org.apache.logging.log4j.ThreadContext.clearMap();
        
        logger.error("This is an ERROR message routed through standard LogManager!");
        
        System.out.println("Test complete. Waiting for Rust background thread...");
        Thread.sleep(1000);
    }
}
