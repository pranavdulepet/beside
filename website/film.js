/*! Includes omggif 1.0.10 by Dean McNamee, MIT. See licenses/omggif.txt. */
import {GifReader} from 'omggif';

function clock(seconds) {
  const whole = Math.floor(seconds);
  return Math.floor(whole / 60) + ':' + String(whole % 60).padStart(2, '0');
}

async function readCaptions(url, signal) {
  const response = await fetch(url, {signal});
  if (!response.ok) throw new Error('Captions unavailable');
  const text = (await response.text()).replace(/\r/g, '');
  const seconds = stamp => stamp.trim().split(':').reduce((v, part) => v * 60 + Number(part), 0);
  return text.trim().split(/\n\s*\n/).slice(1).flatMap(block => {
    const lines = block.split('\n');
    if (/^NOTE(?:\s|$)/.test(lines[0])) return [];
    const timing = lines[0].includes('-->') ? 0 : 1;
    const stamps = lines[timing]?.match(/^((?:\d{2,}:)?\d{2}:\d{2}\.\d{3})\s+-->\s+((?:\d{2,}:)?\d{2}:\d{2}\.\d{3})(?:\s|$)/);
    if (!stamps) return [];
    return [{start: seconds(stamps[1]), end: seconds(stamps[2]), text: lines.slice(timing + 1).join(' ')}];
  });
}

// Each recording is an opaque first image followed by retained GIF patches.
// Other GIF disposal modes can still be played by the browser's image element.
export function playFilm(film, bytes, {label, captionsUrl, startAt, signal}) {
  const reader = new GifReader(new Uint8Array(bytes));
  let duration = 0;
  const ends = Array.from({length: reader.numFrames()}, (_, i) => {
    const frame = reader.frameInfo(i);
    if (frame.disposal > 1) throw new Error('Use native image playback');
    return duration += frame.delay / 100;
  });
  if (!duration) throw new Error('Empty recording');

  const canvas = film.querySelector('canvas');
  canvas.width = reader.width;
  canvas.height = reader.height;
  canvas.setAttribute('aria-label', label);
  const context = canvas.getContext('2d', {alpha: false});
  const pixels = context.createImageData(canvas.width, canvas.height);
  const toggle = film.querySelector('.toggle');
  const seek = film.querySelector('input');
  const time = film.querySelector('.film-time');
  const cc = film.querySelector('.captions');
  const cue = film.querySelector('.film-cue');
  const expand = film.querySelector('.expand');
  const dialog = document.querySelector('.cinema');
  const slot = film.parentElement;
  let position = 0, nextFrame = 0, playing = false, origin = 0, animation;
  let captions;

  function updateCue() {
    const current = cc.getAttribute('aria-pressed') === 'true'
      && captions?.find(item => item.start <= position && position < item.end);
    cue.textContent = current?.text || '';
    cue.hidden = !current;
  }

  function draw(seconds) {
    position = Math.max(0, Math.min(duration, seconds));
    // Sequential playback only decodes newly visible patches. Rebuild from the
    // first frame on a backward seek, keeping a single screen of pixels in memory.
    if (nextFrame > 1 && position < ends[nextFrame - 2]) nextFrame = 0;
    let changed = false;
    while (nextFrame < ends.length && (nextFrame === 0 || position >= ends[nextFrame - 1])) {
      reader.decodeAndBlitFrameRGBA(nextFrame++, pixels.data);
      changed = true;
    }
    if (changed) context.putImageData(pixels, 0, 0);
    seek.value = String(position);
    const elapsed = clock(position);
    const text = elapsed + ' / ' + clock(Math.round(duration));
    if (time.textContent !== text) {
      time.textContent = text;
      seek.setAttribute('aria-valuetext', text);
    }
    updateCue();
  }

  function setPlaying(value) {
    cancelAnimationFrame(animation);
    playing = value;
    toggle.setAttribute('aria-label', value ? 'Pause recording' : 'Play recording');
    toggle.querySelector('path').setAttribute('d', value ? 'M4 3h4v14H4zm8 0h4v14h-4z' : 'M5 2.5l12 7.5-12 7.5z');
    if (value) {
      if (position >= duration) draw(0);
      origin = performance.now() - position * 1000;
      animation = requestAnimationFrame(tick);
    }
  }

  function tick(now) {
    draw((now - origin) / 1000);
    if (position >= duration) setPlaying(false);
    else animation = requestAnimationFrame(tick);
  }

  function restoreSize() {
    if (film.parentElement === dialog) slot.prepend(film);
    expand.setAttribute('aria-label', 'Expand recording');
  }
  expand.addEventListener('click', () => {
    if (dialog.open) {
      dialog.close();
      restoreSize();
      expand.focus();
    } else {
      dialog.append(film);
      dialog.showModal();
      expand.setAttribute('aria-label', 'Exit expanded view');
      expand.focus();
    }
  }, {signal});
  dialog.addEventListener('close', restoreSize, {signal});
  toggle.addEventListener('click', () => setPlaying(!playing), {signal});
  seek.max = String(duration);
  seek.addEventListener('input', () => {
    draw(Number(seek.value));
    origin = performance.now() - position * 1000;
  }, {signal});

  cc.setAttribute('aria-pressed', 'false');
  cc.addEventListener('click', async () => {
    const on = cc.getAttribute('aria-pressed') !== 'true';
    cc.setAttribute('aria-pressed', String(on));
    if (on && !captions) {
      try { captions = await readCaptions(captionsUrl, signal); }
      catch { captions = [{start: 0, end: duration, text: 'Captions unavailable.'}]; }
    }
    if (!signal.aborted) updateCue();
  }, {signal});
  document.addEventListener('visibilitychange', () => {
    if (document.hidden && playing) setPlaying(false);
  }, {signal});
  signal.addEventListener('abort', () => {
    cancelAnimationFrame(animation);
    if (dialog.open) dialog.close();
    restoreSize();
    film.hidden = true;
  }, {once: true});

  draw(startAt);
  film.hidden = false;
  setPlaying(true);
}
