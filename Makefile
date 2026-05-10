TARGET  := riscv64imac-unknown-none-elf
FEATURE := qemu-virt

APPS := $(patsubst apps/%/,%,$(sort $(dir $(wildcard apps/*/Cargo.toml))))

# make <app>       → 运行该模块全部 bin
# make <app>.<bin> → 运行指定 bin

define run_bin
	@printf "%-25s " "$(1)"; \
	cargo run --bin $(1) --features $(FEATURE) --target $(TARGET) -p $(2) -q \
		2> /tmp/rt-async-$(1).log \
		&& printf "\033[32mPASS\033[0m\n" || { printf "\033[31mFAIL\033[0m\n"; cat /tmp/rt-async-$(1).log; }
endef

define bin_target
.PHONY: $(1).$(2)
$(1).$(2):
	$$(call run_bin,$(2),$(1))
endef

$(foreach app,$(APPS),$(foreach bin,$(notdir $(basename $(wildcard apps/$(app)/src/bin/*.rs))),$(eval $(call bin_target,$(app),$(bin)))))

.PHONY: $(APPS)

$(APPS):
	@for b in $(notdir $(basename $(wildcard apps/$@/src/bin/*.rs))); do \
		printf "%-25s " "$$b"; \
		cargo run --bin $$b --features $(FEATURE) --target $(TARGET) -p $@ -q \
			> /dev/null 2> /tmp/rt-async-$$b.log \
			&& printf "\033[32mPASS\033[0m\n" || { printf "\033[31mFAIL\033[0m\n"; cat /tmp/rt-async-$$b.log; }; \
	done
