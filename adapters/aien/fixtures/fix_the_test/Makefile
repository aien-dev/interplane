CC ?= cc
CFLAGS ?= -Wall -Wextra -O0

build/test_clamp: src/clamp.c src/clamp.h test/test_clamp.c
	mkdir -p build
	$(CC) $(CFLAGS) -o $@ src/clamp.c test/test_clamp.c

.PHONY: test clean
test: build/test_clamp
	./build/test_clamp

clean:
	rm -rf build
