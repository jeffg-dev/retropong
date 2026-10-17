.DEFAULT_GOAL := help

.PHONY: help flash calibrate run swap

help:
	@echo "make flash      Build and install the controller firmware (first-time setup or updates)"
	@echo "make calibrate  Learn both knobs' full range and prompt to save it"
	@echo "make run        Build and play RetroPong locally"
	@echo "make swap       Flip and save left/right wheel assignment"

flash:
	rustup target add thumbv8m.main-none-eabihf
	cargo run -p rcctl -- flash

calibrate:
	cargo run -p rcctl -- calibrate

run:
	cargo run -p superpong --release

swap:
	cargo run -p rcctl -- cal swap
