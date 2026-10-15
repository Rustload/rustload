<script lang="ts">
  /**
   * Decorative: a file filling up through several connections at once, the way Rustload
   * downloads. Plays once; static under reduced motion.
   */
  const REGIONS = [1.25, 1.6, 1.05, 1.45, 1.3, 1.7]
</script>

<div class="strip flex h-1.5 w-full gap-px overflow-hidden rounded-full" aria-hidden="true">
  {#each REGIONS as duration, i (i)}
    <div class="relative flex-1 bg-signal-wash">
      <div class="fill absolute inset-y-0 left-0 bg-signal" style:animation-duration="{duration}s"></div>
    </div>
  {/each}
</div>

<style>
  /* Once complete, the strip steps back: the signal colour is for things that are moving. */
  .strip {
    animation: settle 0.6s ease-out 2.1s forwards;
  }
  @keyframes settle {
    to {
      opacity: 0.35;
    }
  }
  .fill {
    width: 100%;
    animation-name: fill;
    animation-delay: 0.15s;
    animation-timing-function: cubic-bezier(0.4, 0.1, 0.3, 1);
    animation-fill-mode: backwards;
  }
  .fill::after {
    content: '';
    position: absolute;
    inset-block: 0;
    right: -1px;
    width: 2px;
    background: var(--fg);
    box-shadow: 0 0 6px 1px var(--signal);
    animation: head-out 0.3s ease-out forwards;
    animation-delay: inherit;
    animation-duration: inherit;
    animation-name: head-out;
  }
  @keyframes fill {
    from {
      width: 0;
    }
  }
  @keyframes head-out {
    0%,
    90% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
</style>
