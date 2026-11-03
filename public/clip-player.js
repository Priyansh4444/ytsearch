// Makes YouTube's player behave like a clip on a shared moment: the timeline covers
// only the window, and the clip ends by rewinding to its start, before YouTube's
// end screen. Without this script the page's iframe still plays from the same start.
(() => {
  const frame = document.getElementById("clip-video");
  const controls = document.getElementById("clip-controls");
  const toggle = document.getElementById("clip-toggle");
  const seek = document.getElementById("clip-seek");
  const clock = document.getElementById("clip-clock");
  if (!frame || !controls || !toggle || !seek || !clock) return;

  const start = Number(frame.dataset.start);
  const end = Number(frame.dataset.end);
  const length = end - start;
  const END_MARGIN_SECONDS = 0.25;
  const TICK_MS = 100;
  let player;
  let timer;
  let seeking = false;

  const label = (seconds) => {
    const whole = Math.max(0, Math.round(seconds));
    return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
  };
  const show = (position) => {
    if (!seeking) seek.value = String(position);
    seek.style.setProperty("--clip-progress", `${(position / length) * 100}%`);
    clock.textContent = `${label(position)} / ${label(length)}`;
  };
  const stopTicking = () => {
    clearInterval(timer);
    timer = undefined;
  };
  const finish = () => {
    stopTicking();
    player.pauseVideo();
    player.seekTo(start, true);
    toggle.textContent = "Replay clip";
    show(0);
  };
  const tick = () => {
    const position = player.getCurrentTime() - start;
    if (position >= length - END_MARGIN_SECONDS) finish();
    else show(Math.max(0, position));
  };

  const onStateChange = (event) => {
    if (event.data === window.YT.PlayerState.PLAYING) {
      // A viewer can click YouTube's own suggestions out of the window; bring it back.
      const position = player.getCurrentTime();
      if (position < start - 1 || position >= end) player.seekTo(start, true);
      toggle.textContent = "Pause";
      timer ??= setInterval(tick, TICK_MS);
    } else if (event.data === window.YT.PlayerState.PAUSED) {
      stopTicking();
      if (toggle.textContent === "Pause") toggle.textContent = "Play clip";
    } else if (event.data === window.YT.PlayerState.ENDED) {
      finish();
    }
  };

  const onReady = () => {
    seek.max = String(length);
    show(0);
    controls.hidden = false;
    toggle.addEventListener("click", () => {
      if (player.getPlayerState() === window.YT.PlayerState.PLAYING) player.pauseVideo();
      else player.playVideo();
    });
    seek.addEventListener("pointerdown", () => {
      seeking = true;
    });
    seek.addEventListener("change", () => {
      seeking = false;
      player.seekTo(start + Number(seek.value), true);
      show(Number(seek.value));
    });
    seek.addEventListener("input", () => show(Number(seek.value)));
    // In a post, X's own play button was the click. Browsers may still refuse this,
    // and then the Play clip button remains.
    if (frame.dataset.autoplay !== undefined) player.playVideo();
  };

  // YouTube's timeline would show the whole video, so its controls go only once this
  // script has loaded and can replace them.
  window.onYouTubeIframeAPIReady = () => {
    const url = new URL(frame.src);
    url.searchParams.set("enablejsapi", "1");
    url.searchParams.set("controls", "0");
    url.searchParams.set("playsinline", "1");
    url.searchParams.set("origin", window.location.origin);
    frame.src = url.toString();
    player = new window.YT.Player(frame, { events: { onReady, onStateChange } });
  };
  const api = document.createElement("script");
  api.src = "https://www.youtube.com/iframe_api";
  document.head.append(api);
})();
