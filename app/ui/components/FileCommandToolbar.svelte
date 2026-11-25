<script lang="ts" module>
  import { createRovingFocus, definePrimitive } from '@ui-sdk/core';

  const fileCommands = definePrimitive({
    name: 'file-commands',
    state: (): { stop: string | null } => ({ stop: null }),
    api: context => createRovingFocus(context, {
      slot: 'command',
      key: 'stop',
      orientation: () => 'horizontal',
      loop: () => true,
      focusable: ({ props }: { props: HTMLButtonElement }) => !props.disabled && !props.hidden,
    }),
    slots: () => ({}),
  });
</script>

<script lang="ts">
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();
  const id = $props.id();

  function attachToolbar(toolbar: HTMLDivElement) {
    const commands = [...toolbar.querySelectorAll<HTMLButtonElement>('button.ribbon-command')];
    const navigation = fileCommands({ id });
    const items = commands.map(command => ({ key: command.id, props: command }));
    const unbind = items.map(item => navigation.api.bind(item.props, item));
    const updateTabStops = () => {
      const { stop } = navigation.get();
      for (const command of commands) {
        const next = command.id === stop ? '0' : '-1';
        if (command.getAttribute('tabindex') !== next) command.setAttribute('tabindex', next);
      }
    };
    const unsubscribe = navigation.subscribe(updateTabStops);
    const observer = new MutationObserver(() => navigation.api.check());
    // The desktop controller owns native disabled and hidden flags on these
    // buttons. Recheck their eligibility without replacing or mirroring them.
    for (const command of commands) observer.observe(command, {
      attributes: true,
      attributeFilter: ['disabled', 'hidden'],
    });
    navigation.api.check();
    updateTabStops();

    function onKeydown(event: KeyboardEvent) {
      if (event.shiftKey || event.isComposing || !(event.target instanceof HTMLButtonElement)) return;
      navigation.api.keydown(event, event.target);
    }
    toolbar.addEventListener('keydown', onKeydown);
    return () => {
      toolbar.removeEventListener('keydown', onKeydown);
      observer.disconnect();
      unsubscribe();
      for (const cleanup of unbind) cleanup();
      navigation.destroy();
    };
  }
</script>

<div class="command-toolbar command-ribbon" role="toolbar" aria-label="File commands" {@attach attachToolbar}>
  {@render children()}
</div>
