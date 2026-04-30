package org.apache.logging.log4j.core;

public class Main {
    public static void main(String[] args) throws Exception {
        System.out.println("Starting Rlog4 Java FFM Bridge test...");
        
        // Log levels: 1=TRACE, 2=DEBUG, 3=INFO, 4=WARN, 5=ERROR
        NativeLogger.log(3, "Hello from Java 22 via FFM to Rust!");
        NativeLogger.log(5, "This is an error message sent bypassing JVM GC allocation pauses.");
        
        System.out.println("Logs sent to Rust engine successfully.");
        
        // Sleep to allow the background Rust thread to print the messages
        Thread.sleep(1000);
    }
}
