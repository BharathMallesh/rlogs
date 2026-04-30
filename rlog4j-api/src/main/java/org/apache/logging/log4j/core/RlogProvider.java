package org.apache.logging.log4j.core;

import org.apache.logging.log4j.spi.Provider;

public class RlogProvider extends Provider {
    public RlogProvider() {
        super(15, "2.6.0", RlogContextFactory.class);
    }
}
