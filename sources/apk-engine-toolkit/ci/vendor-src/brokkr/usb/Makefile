CC ?= clang
CFLAGS ?= -O2 -g
CPPFLAGS += -D_FORTIFY_SOURCE=2
WARNINGS = -std=c11 -Wall -Wextra -Werror -Wpedantic

all: bin/brokkr-usb-broker

bin/brokkr-usb-broker: brokkr_usb_broker.c brokkr_usb_client.c brokkr_usb_client.h
	mkdir -p bin
	$(CC) $(CPPFLAGS) $(CFLAGS) $(WARNINGS) brokkr_usb_broker.c brokkr_usb_client.c -o $@

bin/test-usb-broker: tests/test_usb_broker.c brokkr_usb_client.c brokkr_usb_client.h
	mkdir -p bin
	$(CC) $(CPPFLAGS) $(CFLAGS) $(WARNINGS) -I. tests/test_usb_broker.c brokkr_usb_client.c -o $@

check: all bin/test-usb-broker
	./bin/test-usb-broker ./bin/brokkr-usb-broker

.PHONY: all check
