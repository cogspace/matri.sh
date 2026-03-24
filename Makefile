.PHONY: build debug clean

build:
	cargo build --release
	rm -f target/release/matri.sh
	mv target/release/matrish target/release/matri.sh

debug:
	cargo build
	rm -f target/debug/matri.sh
	mv target/debug/matrish target/debug/matri.sh

clean:
	cargo clean
