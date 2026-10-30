<script lang="ts">
  import { icons, type IconName } from './icons';

  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg class="ico" width={size} height={size} viewBox="0 0 256 256" fill="currentColor" aria-hidden="true" focusable="false">
  {@html icons[name]}
</svg>

<style>
  .ico {
    flex: none;
  }
</style>
