package main

import (
	"fmt"
	"github.com/fatih/color"
)

func main() {
	c := color.New(color.FgGreen)
	c.Println("Hello from Go integration test!")
	fmt.Println("SUCCESS: Go stack is working")
}
