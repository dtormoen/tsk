package com.integ;

import org.apache.commons.lang3.StringUtils;

public class App {
    public static void main(String[] args) {
        String message = StringUtils.capitalize("hello from java integration test!");
        System.out.println(message);
        System.out.println("SUCCESS: Java stack is working");
    }
}
