<script module lang="ts">
  // ponytail: .tgs is gzipped Lottie JSON — DecompressionStream is native, no pako.
  // Cache au niveau MODULE (partagé par toutes les instances) : le picker monte ~100
  // <Tgs>, sinon chaque cellule re-fetch/re-gunzip le même pack (~10 Mo) → jank + RAM.
  const cache = new Map<string, Promise<unknown>>();
  function load(n: string): Promise<unknown> {
    let p = cache.get(n);
    if (!p) {
      p = fetch(`/emoji/${n}.tgs`)
        .then((r) => r.blob())
        .then((b) => new Response(b.stream().pipeThrough(new DecompressionStream("gzip"))).json())
        .catch((e) => { cache.delete(n); throw e; }); // un .tgs KO ne fige pas un rejet à vie dans le cache partagé
      cache.set(n, p);
    }
    return p;
  }
</script>

<script lang="ts">
  import lottie from "lottie-web/build/player/lottie_light_canvas";

  let { name, size = 18, play = true }: { name: string; size?: number; play?: boolean } = $props();

  let host = $state<HTMLDivElement | null>(null);
  let anim = $state<ReturnType<typeof lottie.loadAnimation> | null>(null);

  $effect(() => {
    const el = host;
    if (!el) return;
    let dead = false;
    load(name).then((animationData) => {
      if (dead) return;
      anim = lottie.loadAnimation({ container: el, renderer: "canvas", loop: true, autoplay: false, animationData });
    }).catch(() => {}); // .tgs absent/corrompu : cellule vide, pas de rejet non géré
    return () => { dead = true; anim?.destroy(); anim = null; };
  });

  $effect(() => {
    if (!anim) return;
    if (play) anim.play();
    else anim.goToAndStop(0, true);
  });
</script>

<div bind:this={host} class="tgs" style="width:{size}px;height:{size}px"></div>

<style>
  .tgs {
    flex: none;
    display: grid;
    place-items: center;
  }
  .tgs :global(canvas) {
    width: 100%;
    height: 100%;
  }
</style>
