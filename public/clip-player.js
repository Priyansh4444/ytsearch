// Makes YouTube's player behave like a clip on a shared moment. The timeline covers
// only the window, the transcript follows along, and between plays a still replaces
// the player, because YouTube's pause and end screens fill it with its own buttons.
// The still hides the player rather than covering it: nothing sits on top of a
// visible YouTube player. Without this script the iframe plays from the same start.
(() => {
  const byId = (id) => document.getElementById(id);
  const frame = byId("clip-video");
  const stage = byId("clip-stage");
  const poster = byId("clip-poster");
  const controls = byId("clip-controls");
  const toggle = byId("clip-toggle");
  const mute = byId("clip-mute");
  const seek = byId("clip-seek");
  const clock = byId("clip-clock");
  if (!frame || !stage || !poster || !controls || !toggle || !mute || !seek || !clock) return;

  const start = Number(frame.dataset.start);
  const end = Number(frame.dataset.end);
  const length = end - start;
  const END_MARGIN_SECONDS = 0.25;
  const TICK_MS = 100;
  const AUTOPLAY_GRACE_MS = 4000;
  const PAUSE_SETTLE_MS = 300;
  const list = byId("clip-captions");
  const lines = list ? [...list.querySelectorAll("li")] : [];
  const lineStarts = lines.map((line) => Number(line.dataset.start));
  const restingLine = Math.max(
    0,
    lines.findIndex((line) => line.classList.contains("is-match")),
  );
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let player;
  let timer;
  let seeking = false;
  let wantsPlay = false;
  let settle;
  let currentLine = -1;

  const label = (seconds) => {
    const whole = Math.max(0, Math.round(seconds));
    return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
  };
  const setState = (state) => {
    const playing = state === "playing";
    stage.dataset.state = state;
    toggle.dataset.state = state;
    poster.dataset.state = state;
    poster.hidden = playing;
    const name = playing ? "Pause" : state === "replay" ? "Replay clip" : "Play clip";
    toggle.setAttribute("aria-label", name);
    poster.setAttribute("aria-label", name);
  };
  const showLine = (index) => {
    if (index === currentLine || !lines[index]) return;
    lines[currentLine]?.classList.remove("is-now");
    currentLine = index;
    const line = lines[index];
    line.classList.add("is-now");
    if (list.scrollHeight > list.clientHeight) {
      const top = line.offsetTop - (list.clientHeight - line.offsetHeight) / 2;
      list.scrollTo({ top, behavior: reduceMotion ? "auto" : "smooth" });
    }
  };
  const showLineAt = (seconds) => {
    const ms = seconds * 1000;
    let index = 0;
    for (let i = 0; i < lineStarts.length && lineStarts[i] <= ms; i += 1) index = i;
    showLine(index);
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
    wantsPlay = false;
    player.pauseVideo();
    player.seekTo(start, true);
    setState("replay");
    show(0);
    showLine(restingLine);
  };
  const tick = () => {
    const now = player.getCurrentTime();
    const position = now - start;
    if (position >= length - END_MARGIN_SECONDS) {
      finish();
      return;
    }
    show(Math.max(0, position));
    showLineAt(now);
  };
  const play = () => {
    wantsPlay = true;
    // Show the player first: browsers and YouTube may refuse to play a hidden one.
    stage.dataset.state = "playing";
    poster.hidden = true;
    player.playVideo();
  };
  const seekTo = (position) => {
    player.seekTo(start + position, true);
    show(position);
    showLineAt(start + position);
  };

  const onStateChange = (event) => {
    const states = window.YT.PlayerState;
    clearTimeout(settle);
    if (event.data === states.PLAYING) {
      // Our transcript replaces YouTube's captions, which would repeat it on the video.
      if (lines.length > 0) {
        player.unloadModule("captions");
        player.unloadModule("cc");
      }
      // A viewer can click YouTube's own suggestions out of the window; bring it back.
      const position = player.getCurrentTime();
      if (position < start - 1 || position >= end) player.seekTo(start, true);
      setState("playing");
      timer ??= setInterval(tick, TICK_MS);
    } else if (event.data === states.PAUSED) {
      stopTicking();
      // Seeking pauses briefly, so wait to see whether this pause is the viewer's.
      settle = setTimeout(() => {
        // A finished clip is already showing its replay state.
        if (stage.dataset.state === "playing" && player.getPlayerState() === states.PAUSED) {
          wantsPlay = false;
          setState("poster");
        }
      }, PAUSE_SETTLE_MS);
    } else if (event.data === states.ENDED) {
      finish();
    }
  };

  const onReady = () => {
    seek.max = String(length);
    show(0);
    showLine(restingLine);
    controls.hidden = false;
    const togglePlay = () => {
      if (player.getPlayerState() === window.YT.PlayerState.PLAYING) player.pauseVideo();
      else play();
    };
    toggle.addEventListener("click", togglePlay);
    poster.addEventListener("click", togglePlay);
    mute.addEventListener("click", () => {
      const muted = !player.isMuted();
      if (muted) player.mute();
      else player.unMute();
      mute.dataset.muted = String(muted);
      mute.setAttribute("aria-label", muted ? "Unmute" : "Mute");
    });
    seek.addEventListener("pointerdown", () => {
      seeking = true;
    });
    seek.addEventListener("change", () => {
      seeking = false;
      seekTo(Number(seek.value));
    });
    seek.addEventListener("input", () => show(Number(seek.value)));
    lines.forEach((line, index) => {
      line.querySelector("button")?.addEventListener("click", () => {
        seekTo(Math.max(0, lineStarts[index] / 1000 - start));
        play();
      });
    });
    // In a post, X's own play button was the click. A browser may still refuse, which
    // leaves the still and its play button.
    if (frame.dataset.autoplay !== undefined) {
      play();
      // Only a player that never began was refused; buffering is just a slow start.
      setTimeout(() => {
        const states = window.YT.PlayerState;
        const state = player.getPlayerState();
        if (wantsPlay && (state === states.UNSTARTED || state === states.CUED)) {
          wantsPlay = false;
          setState("poster");
        }
      }, AUTOPLAY_GRACE_MS);
    }
  };

  const image = poster.querySelector("img");
  image?.addEventListener(
    "error",
    () => {
      image.src = `https://i.ytimg.com/vi/${frame.dataset.videoId}/hqdefault.jpg`;
    },
    { once: true },
  );

  // YouTube's timeline would show the whole video, so its controls go only once this
  // script has loaded and can replace them.
  window.onYouTubeIframeAPIReady = () => {
    const url = new URL(frame.src);
    for (const [name, value] of [
      ["enablejsapi", "1"],
      ["controls", "0"],
      ["iv_load_policy", "3"],
      ["playsinline", "1"],
      ["origin", window.location.origin],
    ]) {
      url.searchParams.set(name, value);
    }
    frame.src = url.toString();
    setState("poster");
    player = new window.YT.Player(frame, { events: { onReady, onStateChange } });
  };
  const api = document.createElement("script");
  api.src = "https://www.youtube.com/iframe_api";
  document.head.append(api);
})();
