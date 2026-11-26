<script lang="ts">
  import type { ComponentProps } from 'svelte';
  import { Button } from '@ui-sdk/svelte/button';
  import { buttonVariants, type ButtonSize, type ButtonVariant } from '$lib/components/ui/button/button.svelte';

  let { variant = 'default', size = 'default', class: className, ...props }: Omit<ComponentProps<typeof Button>, 'class'> & {
    variant?: ButtonVariant;
    size?: ButtonSize;
    class?: string;
  } = $props();
</script>

<Button data-slot="button" {...props} data-variant={variant} data-size={size} class={buttonVariants({ variant, size, class: className })}/>
