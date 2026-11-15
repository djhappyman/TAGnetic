<script lang="ts">
  import * as Popover from '$lib/components/ui/popover/index.js';
  import { Button } from '$lib/components/ui/button/index.js';
</script>

<Popover.Root>
  <Popover.Trigger>
    {#snippet child({ props })}
      <Button {...props} variant="ghost" size="icon-sm" class="inspector-help" aria-label="About duplicate scanning">
        <svg class="ico" aria-hidden="true"><use href="#i-info" /></svg>
      </Button>
    {/snippet}
  </Popover.Trigger>
  <Popover.Content class="inspector-help-content duplicate-help-panel" align="start" collisionPadding={12}>
    <h3>Matching methods</h3>
    <dl>
      <dt>Exact copies</dt><dd>Identical files, including tags and artwork.</dd>
      <dt>Same audio</dt><dd>Identical decoded audio. Tags and artwork may differ.</dd>
      <dt>Likely recording</dt><dd>Similar audio fingerprints. Review these matches before choosing a copy.</dd>
    </dl>
    <h3>Reviewing results</h3>
    <p>Select a result to select every copy in its group. Compare the tags in the table and the details in the results box. The search bar narrows scan results while keeping matching copies together.</p>
    <h3>Your files</h3>
    <p>Only the open files are scanned. Scanning does not edit or delete them.</p>
  </Popover.Content>
</Popover.Root>
