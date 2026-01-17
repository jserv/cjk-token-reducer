all:
	cargo build --release

clean:
	cargo clean

indent:
	cargo fmt

install: all
	@./scripts/deploy.sh install

uninstall:
	@./scripts/deploy.sh uninstall

status:
	@./scripts/deploy.sh status
