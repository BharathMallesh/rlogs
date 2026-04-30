package org.apache.logging.log4j.core;

import org.junit.jupiter.api.Test;

public class NativeLoggerTest {

    @Test
    public void testNativeLogging() throws Exception {
        // Log levels: 1=TRACE, 2=DEBUG, 3=INFO, 4=WARN, 5=ERROR
        NativeLogger.log(3, "Hello from Java via FFM to Rust!");
        NativeLogger.log(5, "This is an error message.");
        
        // Sleep to allow the background Rust thread to print the messages
        Thread.sleep(500);
    }
}
