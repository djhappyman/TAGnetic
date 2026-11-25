<script lang="ts">
  import { untrack } from 'svelte';
  import { nextControlId } from '$lib/control-id';
  import Check from '@lucide/svelte/icons/check';
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import { Select, SelectContent, SelectItem, SelectItemIndicator, SelectTrigger } from '@ui-sdk/svelte/select';

  let { native }: { native: HTMLSelectElement } = $props();
  type Choice = { token: string; label: string; description: string; disabled: boolean; hidden: boolean; option: HTMLOptionElement };
  let choices = $state.raw<Choice[]>([]);
  let selectedIndex = $state(-1);
  let disabled = $state(false);
  let name = $state('');
  let title = $state('');
  let trigger = $state<HTMLButtonElement | null>(null);
  let open = $state(false);
  let retainOptions = $state(false);
  let optionGeneration = 0;
  let nativeObserver: MutationObserver | undefined;
  let labels: HTMLLabelElement[] = [];
  const selection = $derived(choices[selectedIndex]);
  const visibleChoices = $derived(choices.filter(choice => !choice.hidden));
  const items = $derived(visibleChoices.map(({ token, label, disabled }) => ({ value: token, label, disabled })));
  const triggerId = untrack(() => `${native.id || nextControlId('select')}-control`);
  const nonLabelContent = '.app-select, select, input, button, textarea';

  function labelText(label: HTMLLabelElement) {
    const copy = label.cloneNode(true) as HTMLLabelElement;
    copy.querySelectorAll(nonLabelContent).forEach(node => node.remove());
    return copy.textContent?.trim() || '';
  }
  function synchronizeOptions() {
    const options = Array.from(native.options);
    if (options.length !== choices.length || options.some((option, index) => option !== choices[index].option)) {
      // A replaced or reordered option list invalidates the open menu's tokens.
      open = false;
      optionGeneration++;
    }
    choices = options.map((option, index) => ({
      option,
      token: `option-${optionGeneration}-${index}`,
      label: option.getAttribute('label') || option.textContent || option.value,
      description: option.dataset.description || '',
      disabled: option.disabled || (option.parentElement instanceof HTMLOptGroupElement && option.parentElement.disabled),
      hidden: option.hidden,
    }));
  }
  function synchronizeName() {
    name = native.getAttribute('aria-label') || labels.map(labelText).join(' ').trim();
  }
  function synchronizePresentation() {
    disabled = native.disabled;
    title = native.title;
    const wrapper = native.closest<HTMLElement>('.app-select');
    if (wrapper) wrapper.hidden = native.hidden;
    if (native.hidden || native.disabled) open = false;
  }
  function synchronizeMutations(records: MutationRecord[]) {
    // Native setters may run inside another component's binding effect. Keep
    // this imperative bridge from subscribing that effect to the state it sets.
    untrack(() => {
      if (records.some(record => record.type !== 'attributes' || (record.target !== native && record.attributeName !== 'selected'))) {
        synchronizeOptions();
      }
      if (records.some(record => record.target === native && record.attributeName === 'aria-label')) synchronizeName();
      if (records.some(record => record.target === native && ['disabled', 'hidden', 'title'].includes(record.attributeName || ''))) {
        synchronizePresentation();
      }
      selectedIndex = native.selectedIndex;
    });
  }
  function synchronizeValue() {
    // Controllers can replace options and set a value in the same turn.
    const records = nativeObserver?.takeRecords();
    if (records?.length) synchronizeMutations(records);
    else selectedIndex = native.selectedIndex;
  }
  function choose(next: string | null) {
    untrack(() => {
      synchronizeValue();
      const index = choices.findIndex(choice => choice.token === next);
      const choice = choices[index];
      if (!choice || native.options[index] !== choice.option || native.hidden || native.disabled || choice.disabled || choice.hidden || native.selectedIndex === index) return;
      native.selectedIndex = index;
      native.dispatchEvent(new Event('input', { bubbles: true }));
      native.dispatchEvent(new Event('change', { bubbles: true }));
      synchronizeValue();
    });
  }
  function dismiss(event: KeyboardEvent) {
    if (event.key !== 'Escape' || !open) return;
    event.preventDefault(); event.stopPropagation(); open = false;
  }
  function attachProxy(host: HTMLSpanElement) {
    host.append(native);
    const originalTabIndex = native.getAttribute('tabindex');
    const originalAria = native.getAttribute('aria-hidden');
    native.tabIndex = -1;
    native.setAttribute('aria-hidden', 'true');
    native.dataset.selectProxy = 'true';
    const saved = new Map<string, PropertyDescriptor | undefined>();
    for (const key of ['value', 'selectedIndex', 'disabled', 'hidden']) {
      const descriptor = Object.getOwnPropertyDescriptor(key === 'hidden' ? HTMLElement.prototype : HTMLSelectElement.prototype, key);
      if (!descriptor?.get || !descriptor.set) continue;
      saved.set(key, Object.getOwnPropertyDescriptor(native, key));
      Object.defineProperty(native, key, { configurable: true, get: () => descriptor.get!.call(native), set: value => { descriptor.set!.call(native, value); synchronizeValue(); } });
    }
    const focus = native.focus;
    native.focus = options => trigger?.focus(options);
    labels = [...native.labels || []];
    const labelTargets = labels.map(label => ({ label, previous: label.htmlFor }));
    // Explicitly target the visible button for both for= labels and wrapping
    // labels, so clicking their text opens the same accessible select.
    const activateLabel = (event: MouseEvent) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target.closest('[data-controller-popup]')) {
        // A top-layer popup still bubbles through its wrapping label.
        // Suppress the label's synthetic trigger click after choosing an option.
        event.preventDefault();
        return;
      }
      if (target.closest('button, input, select, textarea, a') || native.disabled || native.hidden) return;
      // Open directly so a wrapping label cannot deliver a second click.
      event.preventDefault();
      trigger?.focus();
      retainOptions = true;
      open = true;
    };
    labelTargets.forEach(({ label }) => {
      label.htmlFor = triggerId;
      label.addEventListener('click', activateLabel);
    });
    const observer = new MutationObserver(synchronizeMutations);
    nativeObserver = observer;
    observer.observe(native, {
      attributes: true,
      attributeFilter: ['aria-label', 'title', 'disabled', 'hidden', 'label', 'selected', 'value', 'data-description'],
      childList: true,
      subtree: true,
      characterData: true,
    });
    const labelObserver = new MutationObserver(records => {
      const captionChanged = records.some(({ target }) => {
        const element = target instanceof Element ? target : target.parentElement;
        return element && !element.closest(nonLabelContent);
      });
      if (captionChanged) synchronizeName();
    });
    labels.forEach(label => labelObserver.observe(label, { childList: true, subtree: true, characterData: true }));
    native.addEventListener('change', synchronizeValue);
    const form = native.form;
    const reset = () => queueMicrotask(synchronizeValue);
    form?.addEventListener('reset', reset);
    untrack(() => {
      synchronizeOptions();
      synchronizeName();
      synchronizePresentation();
      synchronizeValue();
    });
    return () => {
      observer.disconnect();
      labelObserver.disconnect();
      nativeObserver = undefined;
      native.removeEventListener('change', synchronizeValue);
      form?.removeEventListener('reset', reset);
      native.focus = focus;
      for (const [key, descriptor] of saved) { if (descriptor) Object.defineProperty(native, key, descriptor); else Reflect.deleteProperty(native, key); }
      labelTargets.forEach(({ label, previous }) => {
        label.htmlFor = previous;
        label.removeEventListener('click', activateLabel);
      });
      if (originalTabIndex === null) native.removeAttribute('tabindex'); else native.setAttribute('tabindex', originalTabIndex);
      if (originalAria === null) native.removeAttribute('aria-hidden'); else native.setAttribute('aria-hidden', originalAria);
      delete native.dataset.selectProxy;
    };
  }
</script>

<span class="app-select-proxy" {@attach attachProxy}></span>
<Select bind:open value={selection?.token ?? null} {items} {disabled}
  onOpenChange={next => { if (next) retainOptions = true; }}
  onOpenChangeComplete={next => { if (!next && !open) retainOptions = false; }}
  onValueChange={(next, details) => {
  // Reflect the native select's value after its listeners run. Cancel the SDK
  // write so its hidden form field cannot emit a second input/change pair.
  if (details.reason !== 'reset' && details.reason !== 'restore') choose(next);
  details.cancel();
}}>
  <SelectTrigger id={triggerId} aria-label={name || undefined} {title} class="app-select-trigger" data-proxy-id={native.id} data-state={open ? 'open' : 'closed'} onkeydowncapture={dismiss}>
    {#snippet render({ props })}
      <button type="button" {...props} bind:this={trigger}>
        <span class="app-select-label">{selection?.label}</span>
        <ChevronDown class="size-4 shrink-0 pointer-events-none" aria-hidden="true" />
      </button>
    {/snippet}
  </SelectTrigger>
  <SelectContent class={retainOptions ? 'app-select-menu' : undefined} data-state={open ? 'open' : 'closed'} data-controller-popup align="start" sideOffset={4} collisionPadding={8} onkeydowncapture={dismiss}>
    {#if retainOptions}
      {#each visibleChoices as choice (choice.token)}
        <SelectItem value={choice.token} label={choice.label} disabled={choice.disabled}>
          <span class="app-select-choice"><span>{choice.label}</span>{#if choice.description}<span class="app-select-description">{choice.description}</span>{/if}</span>
          <SelectItemIndicator class="pointer-events-none absolute right-2 flex size-4 items-center justify-center">
            <Check class="size-4" aria-hidden="true" />
          </SelectItemIndicator>
        </SelectItem>
      {/each}
    {/if}
  </SelectContent>
</Select>
