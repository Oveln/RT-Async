TARGET  := riscv64imac-unknown-none-elf
FEATURE := qemu-virt

APPS := $(patsubst apps/%/,%,$(sort $(dir $(wildcard apps/*/Cargo.toml))))

# make <app>         → 运行该模块全部 bin
# make <app> <bin>   → 运行指定 bin
MODULE := $(firstword $(MAKECMDGOALS))
BIN    := $(word 2,$(MAKECMDGOALS))

define run_bin
	@printf "%-25s " "$(1)"; \
	cargo run --bin $(1) --features $(FEATURE) --target $(TARGET) -p $(2) -q \
		> /dev/null 2> /tmp/rt-async-$@.log \
		&& printf "\033[32mPASS\033[0m\n" || { printf "\033[31mFAIL\033[0m\n"; cat /tmp/rt-async-$@.log; }
endef

.PHONY: $(APPS)

$(APPS):
ifneq ($(BIN),)
	$(call run_bin,$(BIN),$@)
else
	@for b in $(notdir $(basename $(wildcard apps/$@/src/bin/*.rs))); do \
		printf "%-25s " "$$b"; \
		cargo run --bin $$b --features $(FEATURE) --target $(TARGET) -p $@ -q \
			> /dev/null 2> /tmp/rt-async-$$b.log \
			&& printf "\033[32mPASS\033[0m\n" || { printf "\033[31mFAIL\033[0m\n"; cat /tmp/rt-async-$$b.log; }; \
	done
endif

# 防止 Make 对 bin 名称报错
$(BIN):
	@true
