<script lang="ts" generics="T extends string | number">
  import { tick } from 'svelte';
  import { scale } from 'svelte/transition';
  import Icon from './Icon.svelte';
  import { ms } from './motion';

  // A themed dropdown that behaves like a native select:
  // click or Enter/Space/Arrow opens, arrows move, Enter picks, Escape closes,
  // Home/End jump, typing a letter jumps to the first match.
  interface Option {
    value: T;
    label: string;
  }
  let {
    value = $bindable(),
    options,
    id,
    label,
    disabled = false,
    onchange,
  }: { value: T; options: Option[]; id?: string; label?: string; disabled?: boolean; onchange?: (value: T) => void } = $props();

  let open = $state(false);
  let active = $state(0);
  let button: HTMLButtonElement;
  let list = $state<HTMLUListElement>();
  const listId = $derived(`${id ?? 'select'}-list`);
  const current = $derived(options.find((o) => o.value === value) ?? options[0]);

  async function show() {
    if (disabled) return;
    open = true;
    active = Math.max(0, options.findIndex((o) => o.value === value));
    await tick();
    list?.focus();
  }

  function pick(i: number) {
    const o = options[i];
    if (o) {
      value = o.value;
      onchange?.(o.value);
    }
    open = false;
    button.focus();
  }

  function onButtonKey(e: KeyboardEvent) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
      e.preventDefault();
      show();
    }
  }

  function onListKey(e: KeyboardEvent) {
    const last = options.length - 1;
    if (e.key === 'ArrowDown') active = Math.min(last, active + 1);
    else if (e.key === 'ArrowUp') active = Math.max(0, active - 1);
    else if (e.key === 'Home') active = 0;
    else if (e.key === 'End') active = last;
    else if (e.key === 'Enter' || e.key === ' ') pick(active);
    else if (e.key === 'Escape' || e.key === 'Tab') {
      open = false;
      if (e.key === 'Escape') button.focus();
      return;
    } else if (e.key.length === 1) {
      const i = options.findIndex((o) => o.label.toLowerCase().startsWith(e.key.toLowerCase()));
      if (i >= 0) active = i;
    } else return;
    e.preventDefault();
    list?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  function onWindowPointer(e: PointerEvent) {
    if (open && !button.parentElement?.contains(e.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={onWindowPointer} />

<div class="select">
  <button
    {id}
    bind:this={button}
    class="trigger slot"
    type="button"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={listId}
    aria-label={label}
    {disabled}
    onclick={() => (open ? (open = false) : show())}
    onkeydown={onButtonKey}
  >
    <span class="num">{current?.label}</span>
    <Icon name="caret-right" size={12} />
  </button>

  {#if open}
    <ul
      id={listId}
      class="menu"
      role="listbox"
      tabindex="-1"
      aria-label={label}
      aria-activedescendant="{listId}-{active}"
      bind:this={list}
      onkeydown={onListKey}
      transition:scale={{ start: 0.96, duration: ms(140), opacity: 0 }}
    >
      {#each options as o, i (o.value)}
        <li
          id="{listId}-{i}"
          data-i={i}
          role="option"
          aria-selected={o.value === value}
          class:active={i === active}
          onpointerenter={() => (active = i)}
          onclick={() => pick(i)}
          onkeydown={() => {}}
        >
          <span class="num">{o.label}</span>
          {#if o.value === value}<Icon name="check" size={14} />{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .select {
    position: relative;
  }
  .trigger {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    text-align: start;
    cursor: pointer;
  }
  .trigger :global(.ico) {
    color: var(--ink-2);
    transform: rotate(90deg);
    transition: transform 0.2s var(--ease);
  }
  .trigger[aria-expanded='true'] :global(.ico) {
    transform: rotate(-90deg);
  }
  .trigger:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .menu {
    position: absolute;
    z-index: 40;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    max-height: 220px;
    overflow: auto;
    margin: 0;
    padding: 4px;
    list-style: none;
    border-radius: 6px;
    background: var(--key);
    box-shadow: inset 0 1px 0 var(--hi), 0 14px 30px -10px var(--shadow), 0 0 0 1px var(--lo);
    transform-origin: top center;
    outline: none;
    scrollbar-width: thin;
    scrollbar-color: var(--ink-3) transparent;
  }
  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 30px;
    padding: 0 8px;
    border-radius: 4px;
    cursor: pointer;
  }
  li.active {
    background: var(--panel);
  }
  li[aria-selected='true'] {
    color: var(--accent);
    font-weight: 600;
  }
</style>
