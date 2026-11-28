import type {Row} from '../src/api';
import {test, expect, type Page} from '@playwright/test';
import {execFileSync, spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {once} from 'node:events';
import {createInterface} from 'node:readline';
import {
  copyFile,
  mkdir,
  truncate,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import {constants} from 'node:fs';
import {fixtureNotebook} from './fixture';
import {basename, dirname, join} from 'node:path';
// Exercise automatic discovery with a predictable installed model on the test Mac.
function withTestModel(rpc: (op: string, args?: object) => Promise<any>) {
  return async (op: string, args: object) => {
    const result = await rpc(op, args);
    return op === 'models'
      ? result.filter((m: {name: string}) => m.name === 'llama3.2:3b')
      : result;
  };
}

async function openPredictions(page: Page) {
  await page.getByRole('button', {name: 'Memory', exact: true}).click();
  await page.getByText('How well Beside guessed', {exact: true}).click();
  await page.getByRole('button', {name: 'Read the predictions'}).click();
}

test('brushstroke, real local prediction, correction, history and persisted sheets', async ({
  page,
}, info) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await page.exposeFunction('besideTestIPC', withTestModel(rpc));
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page
      .getByRole('button', {name: 'Use a model on this Mac', exact: true})
      .click();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await expect(page.getByText('A mark of your own.')).toBeVisible();
    const box = await page
      .getByRole('img', {name: /Draw your personal/})
      .boundingBox();
    if (!box) throw Error('Missing drawing surface');
    await page.mouse.move(box.x + 35, box.y + 70);
    await page.mouse.down();
    await page.mouse.move(box.x + 180, box.y + 90, {steps: 16});
    await page.mouse.up();
    await page.getByRole('button', {name: 'Keep my mark'}).click();
    await page.getByRole('button', {name: 'Teach a correction'}).click();
    await expect(page.getByLabel('Your correction')).toBeVisible();
    await expect(
      page.getByRole('button', {name: /export|chat history/i}),
    ).toHaveCount(0);
    await page
      .getByLabel('Your correction')
      .fill(
        'For a print proof, I asked for uncoated paper so I could mark it in pencil.',
      );
    await page.getByRole('button', {name: 'Read my correction'}).click();
    const learned = page.getByLabel('What Beside learned');
    await expect(learned).toHaveValue(/paper|pencil/i, {timeout: 90000});
    expect((await rpc('state')).entries).toHaveLength(0);
    const rule =
      'For pencil proofing, I always choose uncoated paper over glossy paper.';
    await learned.fill(rule);
    await page.getByRole('button', {name: 'Keep this sentence'}).click();
    await expect(page.getByText(rule, {exact: true})).toBeVisible();
    await page.getByRole('button', {name: 'Return to my notebook'}).click();
    await expect(page.getByRole('button', {name: 'Memory', exact: true})).toHaveAttribute('aria-current', 'page');
    expect((await rpc('state')).firstCorrection.kept).toBe(true);
    await page.getByRole('button', {name: 'Continue your correction'}).click();
    await expect(page.getByLabel('Your next decision')).toBeVisible();
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    await page.getByRole('button', {name: 'Continue your correction'}).click();
    await expect(page.getByText(rule, {exact: true})).toBeVisible();
    await page.goto('/');
    await expect(page.getByLabel('Your next decision')).toBeVisible();
    await page.reload();
    await page
      .getByLabel('Your next decision')
      .fill('For the pencil proof, should I order glossy or uncoated paper?');
    await page.getByRole('button', {name: 'Make a guess'}).click();
    await expect(page.getByLabel('Your decision')).toBeVisible({
      timeout: 90000,
    });
    const hidden = await rpc('state');
    expect(hidden.sealed.guess).toBeUndefined();
    expect(hidden.sealed.ruleIds).toContain(hidden.firstCorrection.id);
    expect(hidden.scores).toHaveLength(0);
    await page.reload();
    await page.getByLabel('Your decision').fill('Order uncoated paper.');
    await page.getByRole('button', {name: 'Open the seal'}).click();
    await expect(page.getByText('The sentence Beside read')).toBeVisible();
    expect((await rpc('state')).sealed.guess).toMatch(/uncoated/i);
    await page.screenshot({path: info.outputPath('correction-reveal.png')});
    await page.getByRole('button', {name: 'It matched'}).click();
    await page.getByRole('button', {name: 'Open the Desk'}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    const completed = await rpc('state');
    expect(completed.entries).toHaveLength(1);
    expect(completed.firstCorrection).toBeNull();
    await page.getByRole('button', {name: 'Teach a correction'}).click();
    await expect(page.getByLabel('Your correction')).toHaveValue('');
    await page.getByRole('button', {name: 'Return to my notebook'}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    expect(completed.sources).toHaveLength(0);
    expect(completed.requestedExport).toBeNull();
    await rpc('newSheet');
    await page.reload();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I keep the meeting brief or make it detailed');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page.getByLabel('Your decision').fill('Keep it brief.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    await expect(page.getByText('You decided · Keep it brief.')).toBeVisible();
    await page.getByRole('button', {name: 'It missed'}).click();
    await openPredictions(page);
    await expect(
      page.getByText('1 of 2 scored guesses matched.'),
    ).toBeVisible();
    await page.getByRole('button', {name: '← Memory'}).click();
    await page.getByRole('button', {name: 'Write an entry'}).click();
    await page
      .getByLabel('Your sentence')
      .fill('I choose short meetings when the agenda is clear.');
    await page.getByRole('button', {name: 'Save to memory'}).click();
    await page
      .getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      })
      .click();
    await page.getByRole('button', {name: 'Edit', exact: true}).click();
    await page
      .getByLabel('Correct this entry')
      .fill('I prefer an agenda before accepting a meeting.');
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    const range = page.getByLabel('Scroll history');
    await range.focus();
    await range.press('End');
    await range.press('ArrowLeft');
    await range.press('ArrowLeft');
    await expect(
      page.getByRole('button', {
        name: 'I choose short meetings when the agenda is clear.',
      }),
    ).toBeVisible();
    await range.press('End');
    await range.blur();
    await expect(
      page.getByRole('button', {
        name: 'I prefer an agenda before accepting a meeting.',
      }),
    ).toBeVisible();
    await page.screenshot({
      path: info.outputPath('scroll.png'),
      animations: 'disabled',
    });
    await page.getByRole('button', {name: 'Connections', exact: true}).click();
    await expect(
      page.getByRole('heading', {name: 'Connected tools'}),
    ).toBeVisible();
    await page.reload();
    await expect(page.getByRole('heading', {name: 'Connected tools'})).toBeVisible();
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(/Should I keep/);
    const sheet = (await rpc('state')).current + '.md';
    expect(await readFile(join(folder, 'desk', sheet), 'utf8')).toContain(
      'Should I keep',
    );
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain('I prefer an agenda');
    expect(
      (await readFile(join(folder, '.beside/moments.db')))
        .subarray(0, 16)
        .toString(),
    ).not.toContain('SQLite format');
  } finally {
    core.stdin.end();
  }
});

test('first reading shows a recent reply and leaves older history for idle time', async ({
  page,
}) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    // Keep this whole conversation in the permanent evaluation split.
    let group = 'paper-review';
    while (createHash('sha256').update(group).digest()[0] >= 26) group += 'x';
    const turn = (sender: string, text: string, daysAgo: number) => ({
      sender,
      text,
      created_at: new Date(Date.now() - daysAgo * 86400000).toISOString(),
    });
    await writeFile(
      join(folder, 'conversations.json'),
      JSON.stringify([
        {
          uuid: group,
          chat_messages: [
            turn('assistant', 'Which typeface for the old poster?', 160),
            turn('human', 'Use the tall lettering.', 160),
            turn('assistant', 'Which paper for the new print?', 2),
            turn('human', 'Use the cream sheet.', 2),
          ],
        },
      ]),
    );
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await page.exposeFunction('besideTestIPC', withTestModel(rpc));
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page
      .getByRole('button', {name: 'Use a model on this Mac', exact: true})
      .click();

    await expect(page.getByText('Ready to think on this Mac.')).toBeVisible();
    // Simulate a reading already requested on a previous build.
    await rpc('pickSource', {kind: 'export'});
    await rpc('beginFirstReading', {notify: false});
    await page.reload();
    await expect(
      page.getByText('Beside hid 1 reply before making its guess.'),
    ).toBeVisible({timeout: 90000});
    await page
      .getByText('Which paper for the new print?', {exact: true})
      .click();
    await expect(
      page.getByText('You said · Use the cream sheet.'),
    ).toBeVisible();
    await expect(page.getByText(/will continue this reading/)).toBeVisible();
    await expect(
      page.getByText('Which typeface for the old poster?'),
    ).toHaveCount(0);
    await page.reload();
    await expect(page.getByText(/will continue this reading/)).toBeVisible();
  } finally {
    core.stdin.end();
  }
});

test('first reading survives a reloaded window and resumes a scored notebook after quit', async ({
  page,
  context,
}) => {
  let notebook = await fixtureNotebook();
  const {folder} = notebook;
  try {
    let group = 'evening-edits';
    while (createHash('sha256').update(group).digest()[0] >= 26) group += 'x';
    const choices = [
      ['Which pen for the cover?', 'Use the fine nib.'],
      ['When should we review the proof?', 'Tomorrow morning.'],
      ['What should the cover show?', 'One printed leaf.'],
      ['Where should the title go?', 'Below the picture.'],
      ['Which paper for the insert?', 'The cream sheet.'],
      ['How many proofs should we send?', 'Just one.'],
    ];
    await writeFile(
      join(folder, 'conversations.json'),
      JSON.stringify([
        {
          uuid: group,
          chat_messages: choices.flatMap(([question, answer]) => [
            {sender: 'assistant', text: question},
            {
              sender: 'human',
              text: answer,
              created_at: new Date(Date.now() - 86400000).toISOString(),
            },
          ]),
        },
      ]),
    );
    await notebook.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await notebook.rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await context.exposeFunction('besideTestIPC', (op: string, args: object) =>
      notebook.rpc(op, args),
    );
    await context.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await notebook.rpc('pickSource', {kind: 'export'});
    await notebook.rpc('beginFirstReading', {notify: false});
    await page.reload();
    await expect(
      page.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    const job = (await notebook.rpc('firstReadingProgress')).job;
    await page.reload();
    await expect(
      page.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    await notebook.rpc('beginFirstReading', {notify: false});
    expect((await notebook.rpc('firstReadingProgress')).job.started).toBe(
      job.started,
    );
    await expect
      .poll(async () => (await notebook.rpc('state')).scores.length, {
        timeout: 60000,
      })
      .toBeGreaterThan(0);
    const before = await notebook.rpc('state');
    expect(before.reading).toBeNull();
    expect(before.scores.length).toBeLessThan(choices.length);
    await page.close();
    const exited = new Promise<void>(resolve =>
      notebook.core.once('close', () => resolve()),
    );
    notebook.core.stdin.end();
    await exited;
    notebook = await fixtureNotebook(false, undefined, folder);
    const reopened = await context.newPage();
    await reopened.goto('/');
    await expect(
      reopened.getByRole('heading', {name: 'Reading your words.'}),
    ).toBeVisible();
    expect((await notebook.rpc('firstReadingProgress')).job.started).toBe(
      job.started,
    );
    await expect(
      reopened.getByText('Beside hid 6 replies before making its guesses.'),
    ).toBeVisible({timeout: 90000});
    const after = await notebook.rpc('state');
    expect(after.firstReading).toBeNull();
    expect(after.scores).toHaveLength(choices.length);
    for (const scored of before.scores)
      expect(after.scores).toContainEqual(scored);
    await reopened.getByRole('button', {name: 'Open the Desk'}).click();
    await expect(reopened.getByLabel('Desk sheet')).toBeVisible();
  } finally {
    notebook.core.stdin.end();
  }
});

test('a requested export arrives after reopening and waits for consent', async ({
  page,
}) => {
  let notebook = await fixtureNotebook();
  const {folder} = notebook;
  const downloads = join(folder, 'Downloads');
  try {
    await mkdir(downloads);
    await notebook.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await notebook.rpc('finish');
    await page.exposeFunction('besideTestIPC', (op: string, args: object) =>
      notebook.rpc(op, args),
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/#chat');
    await page.getByLabel('Desk sheet').fill('Keep this draft open.');
    await expect
      .poll(async () => (await notebook.rpc('state')).sheet)
      .toBe('Keep this draft open.');
    await page.keyboard.press('Meta+,');
    await page.getByText('Web search and imports', {exact: true}).click();
    await page.getByRole('button', {name: 'Request ChatGPT history ↗'}).click();
    await expect(
      page.getByText('Waiting for your chat export in Downloads.'),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Stop looking'}).click();
    await expect(page.getByRole('button', {name: 'Stop looking'})).toHaveCount(
      0,
    );
    await page.getByRole('button', {name: 'Request ChatGPT history ↗'}).click();
    await expect(
      page.getByRole('button', {name: 'Stop looking'}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    notebook.core.stdin.end();
    await new Promise<void>(resolve =>
      notebook.core.once('close', () => resolve()),
    );
    let group = 'print-review';
    while (createHash('sha256').update(group).digest()[0] < 52) group += 'x';
    const part = join(folder, 'conversations-01.json');
    await writeFile(
      part,
      JSON.stringify([
        {
          uuid: group,
          chat_messages: [
            {sender: 'assistant', text: 'Which paper feels right?'},
            {
              sender: 'human',
              text: 'The cream sheet.',
              created_at: new Date().toISOString(),
            },
          ],
        },
      ]),
    );
    const filename = 'cafe'.repeat(16) + '-history.ZIP';
    const archive = join(downloads, filename);
    execFileSync('/usr/bin/zip', ['-q', '-j', archive, part]);
    await rename(archive, archive + '.download');
    notebook = await fixtureNotebook(false, undefined, folder);
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(
      'Keep this draft open.',
    );
    const offer = page.getByRole('button', {
      name: `Add ${filename} to Beside`,
    });
    await expect(offer).toHaveCount(0);
    await rename(archive + '.download', archive);
    // Returning to the window refreshes the notebook just as in the native app.
    await expect(async () => {
      await page.evaluate(() => window.dispatchEvent(new Event('focus')));
      await expect(offer).toBeVisible({timeout: 800});
    }).toPass({timeout: 15000});
    expect((await notebook.rpc('state')).sources).toEqual([]);
    expect((await notebook.rpc('read')).training).toBe(0);
    await page.setViewportSize({width: 520, height: 560});
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(520);
    await offer.click();
    await expect(
      page.getByText(`Added ${filename} to your sources.`),
    ).toBeVisible();
    await expect(page.getByLabel('Desk sheet')).toHaveValue(
      'Keep this draft open.',
    );
    await page.keyboard.press('Meta+,');
    await page.getByText('Web search and imports', {exact: true}).click();
    await page.getByRole('button', {name: 'Import selected sources'}).click();
    await expect
      .poll(async () => (await notebook.rpc('state')).reading?.training)
      .toBe(1);
    const state = await notebook.rpc('state');
    expect(state.requestedExport).toBeNull();
    expect(state.sources.map((s: {path: string}) => s.path)).toEqual([archive]);
  } finally {
    notebook.core.stdin.end();
  }
});

test.describe('website walkthroughs', () => {
  const demos = [
    ['Writing', 'writing'],
    ['Flight', 'baggage'],
    ['Guidance', 'guidance'],
    ['Agents', 'agents'],
  ];
  async function choose(page: import('@playwright/test').Page, title: string) {
    const tab = page.getByRole('tab', {name: title, exact: true});
    await tab.click();
    await expect(tab).toHaveAttribute('aria-selected', 'true');
  }
  test('the complete gallery fits one row on a narrow phone and supports keyboard and shared links', async ({page}) => {
    await page.setViewportSize({width: 320, height: 640});
    await page.goto('/website/index.html');
    const choices = page.getByRole('tab');
    await expect(choices).toHaveCount(demos.length);
    expect(await choices.allTextContents()).toEqual(demos.map(([name]) => name));
    const rects = await choices.evaluateAll(elements => elements.map(el => {
      const {left, right, top, height} = el.getBoundingClientRect();
      return {left, right, top, height};
    }));
    expect(new Set(rects.map(rect => rect.top)).size).toBe(1);
    expect(Math.min(...rects.map(rect => rect.height))).toBeGreaterThanOrEqual(44);
    expect(Math.min(...rects.map(rect => rect.left))).toBeGreaterThanOrEqual(0);
    expect(Math.max(...rects.map(rect => rect.right))).toBeLessThanOrEqual(320);
    const first = page.getByRole('tab', {name: 'Writing'});
    await first.focus();
    await page.keyboard.press('End');
    await expect(page.getByRole('tab', {name: 'Agents'})).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect(first).toBeFocused();
    await page.keyboard.press('ArrowLeft');
    await expect(page).toHaveURL(/#agents$/);
    await page.reload();
    await expect(page.getByRole('button', {name: 'Play Agents walkthrough'})).toBeVisible();
  });

  test('desktop recordings play, seek and reopen the selected tab', async ({
    page,
  }) => {
    await page.goto('/website/index.html');
    const player = page.locator('video');
    for (const [title, file] of demos) {
      await choose(page, title);
      await expect(player.locator('track')).toHaveCount(0);
      await page
        .getByRole('button', {name: `Play ${title} walkthrough`})
        .click();
      await expect
        .poll(() =>
          player.evaluate(
            (v: HTMLVideoElement, slug: string) =>
              !!v.currentSrc &&
              new URL(v.currentSrc).pathname.endsWith('/' + slug + '.mp4') &&
              !v.paused &&
              !v.error &&
              v.defaultPlaybackRate === 1 &&
              v.playbackRate === 1 &&
              v.currentTime > 0.15 &&
              v.getVideoPlaybackQuality().totalVideoFrames > 1,
            file,
          ),
        )
        .toBe(true);
      const frames = await player.evaluate((v: HTMLVideoElement) => {
        v.currentTime = v.duration / 2;
        return v.getVideoPlaybackQuality().totalVideoFrames;
      });
      await expect
        .poll(() =>
          player.evaluate((v: HTMLVideoElement) =>
            !v.seeking && v.currentTime > v.duration / 2
              ? v.getVideoPlaybackQuality().totalVideoFrames
              : 0,
          ),
        )
        .toBeGreaterThan(frames);
      await player.evaluate((v: HTMLVideoElement) => {
        v.currentTime = Math.max(0, v.duration - 0.5);
      });
      await expect
        .poll(() =>
          player.evaluate(
            (v: HTMLVideoElement) =>
              v.ended &&
              !v.error &&
              Math.abs(v.duration - v.currentTime) < 0.08,
          ),
        )
        .toBe(true);
    }
    await page.reload();
    await expect(
      page.getByRole('button', {
        name: 'Play Agents walkthrough',
      }),
    ).toBeVisible();
  });

  test('a rejected desktop play request automatically shows the same recording as images', async ({
    page,
  }) => {
    await page.addInitScript(() => {
      HTMLMediaElement.prototype.play = () =>
        Promise.reject(new DOMException('Media blocked', 'NotAllowedError'));
    });
    await page.goto('/website/index.html#guidance');
    const play = page.getByRole('button', {name: 'Play Guidance walkthrough'});
    await play.focus();
    await page.keyboard.press('Enter');
    await expect(
      page.getByRole('button', {name: 'Pause recording'}),
    ).toBeFocused();
    await expect(
      page.getByRole('img', {name: 'Guidance walkthrough'}),
    ).toBeVisible();
    await expect
      .poll(() => page.getByRole('slider').inputValue().then(Number))
      .toBeGreaterThan(0.2);
  });

  test.describe('with JavaScript disabled', () => {
    test.use({javaScriptEnabled: false});
    test('every recording opens directly', async ({page}) => {
      await page.goto('/website/index.html');
      await expect(page.getByRole('tab')).toHaveCount(0);
      for (const [name, file] of demos) {
        await expect(
          page.getByRole('link', {name, exact: true}),
        ).toHaveAttribute('href', 'media/' + file + '.mp4');
      }
    });
  });

  test.describe('phone playback', () => {
    test.use({
      hasTouch: true,
      isMobile: true,
      viewport: {width: 393, height: 852},
    });

    test('every full recording still plays and seeks when the native video fails', async ({
      page,
    }) => {
      let videoRequests = 0;
      await page.route('**/*.mp4*', route => {
        videoRequests++;
        return route.abort();
      });
      await page.goto('/website/index.html');
      const seek = page.getByRole('slider', {name: 'Recording position'});
      for (const [title] of demos) {
        await choose(page, title);
        await page
          .getByRole('button', {name: 'Play ' + title + ' walkthrough'})
          .tap();
        await expect
          .poll(() => seek.inputValue().then(Number))
          .toBeGreaterThan(0.15);
        await page.getByRole('button', {name: 'Pause recording'}).tap();
        await expect(page.getByRole('button', {name: 'Captions'})).toHaveCount(0);
        const image = page.getByRole('img', {name: title + ' walkthrough'});
        const start = await image.evaluate((c: HTMLCanvasElement) =>
          c.toDataURL(),
        );
        const end = Number(await seek.getAttribute('max'));
        await seek.click();
        const middle = await image.evaluate((c: HTMLCanvasElement) =>
          c.toDataURL(),
        );
        await page.keyboard.press('End');
        await expect
          .poll(() => seek.inputValue().then(Number))
          .toBeCloseTo(end, 1);
        const last = await image.evaluate((c: HTMLCanvasElement) =>
          c.toDataURL(),
        );
        expect(new Set([start, middle, last]).size).toBeGreaterThan(1);
        await page.keyboard.press('Home');
        await page.getByRole('button', {name: 'Play recording'}).tap();
      }
      expect(videoRequests).toBeGreaterThanOrEqual(demos.length);
      await page.getByRole('button', {name: 'Expand recording'}).tap();
      await expect(
        page.getByRole('dialog', {name: 'Expanded walkthrough'}),
      ).toBeVisible();
      await page.getByRole('button', {name: 'Exit expanded view'}).tap();
      await expect(page.getByRole('dialog')).toBeHidden();
      await page.reload();
      await expect(
        page.getByRole('button', {
          name: 'Play Agents walkthrough',
        }),
      ).toBeVisible();
    });

    test('a failed media host offers the selected recording from the repository', async ({
      page,
    }) => {
      await page.route(/\.(?:mp4|gif)(?:\?|$)/, route => route.abort());
      await page.goto('/website/index.html#flight');
      await page
        .getByRole('button', {name: 'Play Flight walkthrough'})
        .tap();
      const backup = page.getByRole('link', {name: 'Download the MP4'});
      await expect(backup).toBeVisible();
      await expect(backup).toHaveAttribute(
        'href',
        'https://github.com/pranavdulepet/beside/raw/refs/heads/main/website/media/baggage.mp4',
      );
      await choose(page, 'Writing');
      await expect(backup).toBeHidden();
    });

    test('the original animation opens as an ordinary image if scripted loading is blocked', async ({
      page,
    }) => {
      let videoRequests = 0;
      await page.route('**/*.mp4*', route => {
        videoRequests++;
        return route.abort();
      });
      await page.route(/\.gif(?:\?|$)/, route =>
        route.request().resourceType() === 'fetch'
          ? route.abort()
          : route.continue(),
      );
      await page.goto('/website/index.html?player=images#guidance');
      await page.getByRole('button', {name: 'Play Guidance walkthrough'}).tap();
      const animation = page.locator('img.direct-recording');
      await expect(animation).toBeVisible();
      expect(videoRequests).toBe(0);
      await expect(animation).toHaveAttribute(
        'src',
        /^media\/guidance\.gif(?:\?|$)/,
      );
      expect(
        await animation.evaluate((img: HTMLImageElement) => img.naturalWidth),
      ).toBe(1024);
      await page.getByRole('tab', {name: 'Writing'}).tap();
      await expect(animation).toBeHidden();
    });
  });
});

test('paper, brush and downloadable local Mac archive', async ({
  page,
}, info) => {
  await page.goto('/website/index.html');
  await expect(page.getByRole('heading', {name: 'Beside'})).toBeVisible();
  await page.mouse.move(80, 80);
  await page.mouse.down();
  await page.mouse.move(165, 105, {steps: 12});
  await page.mouse.up();
  const painted = await page.locator('body > canvas').evaluate(el => {
    const canvas = el as HTMLCanvasElement;
    return canvas
      .getContext('2d')!
      .getImageData(0, 0, canvas.width, canvas.height)
      .data.some((n, i) => i % 4 === 3 && n > 0);
  });
  expect(painted).toBe(true);
  const download = page.waitForEvent('download');
  await page.getByRole('link', {name: /Download for Mac/}).click();
  const result = await download;
  expect(result.suggestedFilename()).toBe('Beside-macOS.zip');
  expect(await result.failure()).toBeNull();
  const bytes = await readFile((await result.path())!);
  expect(bytes.subarray(0, 4).toString('hex')).toBe('504b0304');
  expect(bytes.includes(Buffer.from('Beside.app/Contents/MacOS/beside'))).toBe(
    true,
  );
  await page.emulateMedia({colorScheme: 'dark', reducedMotion: 'reduce'});
  await page.setViewportSize({width: 520, height: 480});
  await page.screenshot({
    path: info.outputPath('website-dark.png'),
    animations: 'disabled',
  });
});

test('Mirror separates paired blind results by prediction month and model', async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date(2026, 6, 5, 12));
  await page.addInitScript(() => {
    const cases = [
      ['may-win', '2026-05-20', 'held-out', 'A', 0, 2],
      ['may-both', '2026-05-21', 'held-out', 'A', 2, 3],
      ['may-loss', '2026-05-23', 'held-out', 'B', 3, 0],
      ['may-other-grader', '2026-05-24', 'held-out', 'B', 2, 3],
      ['june-tie', '2026-06-04', 'held-out', 'A', 0, 1],
      ['live', '2026-06-04', 'live', 'A', 3, 3],
      ['unpaired', '2026-06-04', 'held-out', 'A', null, 2],
    ];
    const scores = cases.map(([id, t, scope, model, coldGrade, grade]) => ({
      id,
      predictedAt: t + 'T12:00:00Z',
      date: '2026-07-03T12:00:00Z',
      scope,
      model,
      grader: id === 'may-other-grader' ? 'Z' : undefined,
      coldGrade,
      grade,
      category: 'words',
      reflection: 'reviewed',
    }));
    const state = {
      setup: true,
      current: 'sheet',
      sheet: '',
      sheets: [],
      scores,
      log: [],
      entries: [
        {
          text: 'Sketch before choosing a typeface.',
          updated: '2026-07-04T12:00:00Z',
        },
        {text: 'Use an older idea.', updated: '2026-06-21T12:00:00Z'},
        {text: 'A later entry.', updated: '2026-07-06T12:00:00Z'},
      ],
      routes: [],
      tuning: [],
      versions: [],
      proposals: [],
      reading: {tested: 6},
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (cmd: string, p: any) =>
        cmd === 'notebook' && p.operation === 'state' ? state : 1,
    };
  });
  await page.goto('/');
  await page.getByRole('button', {name: 'Memory', exact: true}).click();
  await page.getByText('How well Beside guessed', {exact: true}).click();
  await expect(page.getByText(/Beside hid 6 replies/)).toBeVisible();
  await page.getByRole('button', {name: 'Read the predictions'}).click();
  const note = page.getByRole('region', {name: 'Sunday note'});
  await expect(note).toContainText('one new or revised entry');
  await expect(note).toContainText('Sketch before choosing a typeface.');
  await expect(note).toContainText('5 of 7 scored guesses');
  await expect(note).not.toContainText('A later entry');
  await page.getByText('Does personalization help?').click();
  const table = page.getByRole('table', {
    name: 'Matched guesses by prediction month and model',
  });
  await expect(table.getByRole('row')).toHaveCount(5);
  await expect(table.getByRole('row').nth(1).getByRole('cell')).toHaveText([
    '0/1',
    '0/1',
    '0 pp',
  ]);
  await expect(table.getByRole('row').nth(2).getByRole('cell')).toHaveText([
    '1/2',
    '2/2',
    '+50 pp',
  ]);
  await expect(table.getByRole('row').nth(3).getByRole('cell')).toHaveText([
    '1/1',
    '0/1',
    '-100 pp',
  ]);
  await expect(table.getByRole('row').nth(4)).toContainText('Graded by Z');
  await expect(table.getByRole('row').nth(4).getByRole('cell')).toHaveText([
    '1/1',
    '1/1',
    '0 pp',
  ]);
  await expect(table.getByRole('rowheader').first()).toContainText('Jun');
});

test('Desk distinguishes accepted ink, a written alternative, dismissal and moving the caret', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  const context = 'I think we can call';
  const hint = page.getByText('accepts the light ink', {exact: false});
  const sheet = page.getByLabel('Desk sheet');
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    const models = await rpc('models');
    const model = models.find((m: any) => m.name === 'llama3.2:3b');
    await rpc('model', model);
    await rpc('finish');
    for (let n = 0; n < 2; ) {
      const id = await rpc('newSheet');
      if (createHash('sha256').update(id).digest()[0] < 52) continue;
      await rpc('save', {
        id,
        text: 'What would you say at the end of this meeting?',
      });
      const sealed = await rpc('sealGuess', {id});
      await rpc('breakSeal', {
        id: sealed.id,
        actual: 'We can call this a wrap today.',
      });
      await rpc('verdict', {id: sealed.id, right: false});
      n++;
    }
    await rpc('newSheet');
    let hold = false;
    let release: (() => void) | undefined;
    await page.exposeFunction('besideTestIPC', async (op: string, args: any) => {
      const result = await rpc(op, args);
      if (op === 'ghost' && hold)
        await new Promise<void>(r => {
          release = r;
        });
      return result;
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    await sheet.evaluate(el => {
      el.addEventListener(
        'input',
        () => {
          const start = performance.now();
          const observer = new MutationObserver(() => {
            if (document.querySelector('.ghost-layer')) {
              el.setAttribute(
                'data-display-ms',
                String(Math.round(performance.now() - start)),
              );
              observer.disconnect();
            }
          });
          observer.observe(document.body, {childList: true, subtree: true});
        },
        {once: true},
      );
    });
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    console.log(
      'Desk input to visible ghost ink (ms)',
      await sheet.getAttribute('data-display-ms'),
    );
    await page.screenshot({
      path: info.outputPath('desk-writing.png'),
      animations: 'disabled',
    });
    await sheet.press('Tab');
    await expect(sheet).toHaveValue(context + ' this a wrap today.');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.press('ArrowLeft');
    await expect(hint).toHaveCount(0);
    await sheet.press('Meta+ArrowRight');
    await expect(hint).toBeVisible();
    await sheet.press('Escape');
    await expect(hint).toHaveCount(0);
    await sheet.fill('');
    await sheet.fill(context);
    await expect(hint).toBeVisible();
    await sheet.pressSequentially(' it a rehearsal.', {delay: 12});
    await sheet.blur();
    await expect
      .poll(
        async () =>
          (await rpc('state')).scores.find((v: any) => v.status === 'edited')
            ?.actual,
      )
      .toBe('it a rehearsal.');
    // Discard an inference that returns after the writer selects earlier text.
    hold = true;
    await sheet.fill(context);
    await expect.poll(() => !!release).toBe(true);
    await sheet.press('ArrowLeft');
    hold = false;
    release!();
    await openPredictions(page);
    await page.getByText('Details', {exact: true}).click();
    await page.getByText('How the score is measured').click();
    await expect(
      page.getByText('Ghost ink · 1 accepted of 4 shown.'),
    ).toBeVisible();
    await page.getByText('Prediction record', {exact: true}).click();
    await expect(page.getByText('You wrote · it a rehearsal.')).toBeVisible();
    const rows = (await rpc('state')).scores.filter(
      (v: any) => v.category === 'ghost',
    );
    expect(rows.map((r: any) => r.status).sort()).toEqual([
      'accepted',
      'dismissed',
      'edited',
      'shown',
    ]);
    expect(rows.every((r: any) => r.sources.includes('desk'))).toBe(true);
  } finally {
    core.stdin.end();
  }
});

test('morning return preserves the old sheet and delivers a waiting agent its answer', async ({
  page,
}) => {
  page.setDefaultTimeout(12000);
  const {core, rpc, folder} = await fixtureNotebook(true);
  const configPath = join(folder, 'agent-config', '.claude.json');
  const personal = {
    mcpServers: {beside: {command: 'personal-reader'}},
    theme: 'ink',
  };
  await mkdir(join(folder, 'agent-config'), {recursive: true});
  await writeFile(configPath, JSON.stringify(personal));
  let agent: ReturnType<typeof spawn> | undefined;
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.clock.setFixedTime(new Date(2026, 9, 31, 21));
    await page.goto('/');
    const nav = page.getByRole('navigation', {name: 'Workspace sections'});
    const chat = nav.getByRole('button', {name: 'Chat', exact: true});
    const connections = nav.getByRole('button', {name: 'Connections'});
    await chat.click();
    const sheet = page.getByLabel('Desk sheet');
    const evening = 'I want to finish the sketch tomorrow.';
    await sheet.fill(evening);
    await expect.poll(async () => (await rpc('state')).sheet).toBe(evening);
    await connections.click();
    await page
      .getByRole('button', {name: '+ Claude Code', exact: true})
      .click();
    await page
      .getByRole('button', {name: 'Connect Claude Code', exact: true})
      .click();
    await page
      .locator('summary')
      .filter({hasText: 'Claude Code'})
      .first()
      .click();
    await page
      .getByLabel('Permission', {exact: true})
      .selectOption('Answer for me');
    await expect
      .poll(async () => (await rpc('state')).agents[0]?.permission)
      .toBe('Answer for me');
    await expect(
      page.getByLabel('Line for Claude Code’s instructions'),
    ).toHaveValue(/beside_answer/);
    await expect(page.getByText('MCP server · beside-2')).toBeVisible();
    const installed = await readFile(configPath, 'utf8');
    const config = JSON.parse(installed).mcpServers['beside-2'];
    agent = spawn(config.command, config.args, {
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    const lines = createInterface({input: agent.stdout!})[
      Symbol.asyncIterator
    ]();
    let serial = 0;
    async function ask(name: string, args: Record<string, string | string[]>) {
      agent!.stdin!.write(
        JSON.stringify({
          jsonrpc: '2.0',
          id: ++serial,
          method: 'tools/call',
          params: {name, arguments: args},
        }) + '\n',
      );
      const row = await lines.next();
      if (row.done) throw Error('Agent exited before receiving a result');
      const {result, error} = JSON.parse(row.value);
      if (error) throw Error(error.message);
      if (result.isError) throw Error(result.content[0].text);
      return JSON.parse(result.content[0].text);
    }
    const question = 'Which sketch should I prepare for review?';
    const ticket = await ask('beside_answer', {question});
    expect(ticket.queued).toBe(true);
    for (let i = 0; i < 64; i++)
      await ask('beside_recall', {task: 'Look up review preferences.'});
    expect((await ask('beside_answer', {ticket: ticket.id})).queued).toBe(true);
    await page.clock.setFixedTime(new Date(2026, 10, 1, 8));
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await chat.click();
    await expect(sheet).toHaveValue(evening);
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: '＋ New sheet'}).click();
    await expect(sheet).toHaveValue('');
    await expect(
      page.getByRole('button', {
        name: /1 question from Claude Code is waiting/,
      }),
    ).toBeVisible();
    await sheet.fill('Start with the ink sketch.');
    await expect
      .poll(async () => (await rpc('state')).sheet)
      .toBe('Start with the ink sketch.');
    await page.reload();
    await chat.click();
    await expect(sheet).toHaveValue('Start with the ink sketch.');
    await page
      .getByRole('button', {name: /1 question from Claude Code/})
      .click();
    const answerButton = page.getByRole('button', {
      name: 'Answer question',
      exact: true,
    });
    await expect(answerButton).toBeFocused();
    await answerButton.click();
    await page.getByLabel('Your answer').fill('Use the ink sketch.');
    await page.getByRole('button', {name: 'Keep answer'}).click();
    await expect(
      page.getByText('Use the ink sketch.', {exact: true}),
    ).toBeVisible();
    expect(await ask('beside_answer', {ticket: ticket.id})).toMatchObject({
      answer: 'Use the ink sketch.',
      decidedBy: 'user',
    });
    await chat.click();
    await expect(
      page.getByRole('button', {name: /is waiting for you/}),
    ).toHaveCount(0);
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: new RegExp(evening)}).click();
    await expect(sheet).toHaveValue(evening);
    await page.reload();
    await chat.click();
    await expect(sheet).toHaveValue(evening);
    expect((await rpc('state')).sheets).toHaveLength(2);
    await connections.click();
    await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
    await nav.getByRole('button', {name: 'Memory'}).click();
    await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
    await chat.click();
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: '＋ New sheet'}).click();
    await expect(sheet).toHaveValue('');
    expect((await rpc('state')).sheets).toHaveLength(3);
    await test.step('review earned automatic answers and change this agent’s confidence threshold', async () => {
      const request = {
        question: 'Which format do you prefer?',
        options: ['Bullets', 'Prose'],
      };
      let qualified = 0;
      let automatic = 0;
      for (let i = 0; i < 90; i++) {
        const reply = await ask('beside_answer', request);
        if (!reply.queued) {
          expect(reply.answer).toBe('Bullets');
          automatic++;
          if (reply.confidence >= 0.97 && ++qualified === 20) break;
        }
        await rpc('respond', {id: reply.id, answer: 'Bullets'});
      }
      expect(qualified).toBe(20);
      await connections.click();
      const agentSection = page
        .locator('details')
        .filter({has: page.getByLabel('Permission', {exact: true})});
      await agentSection.locator('summary').first().click();
      await expect(
        agentSection.getByRole('button', {name: 'Raise minimum to 97%'}),
      ).toHaveCount(0);
      const question = page
        .locator('summary')
        .filter({hasText: request.question})
        .first();
      await question.click();
      await page
        .getByRole('button', {name: 'That was right', exact: true})
        .click();
      await expect(agentSection).toContainText(
        automatic +
          ' of ' +
          automatic +
          ' automatic answers reviewed · 0 corrected.',
      );
      await agentSection
        .getByRole('button', {name: 'Raise minimum to 97%'})
        .click();
      await expect
        .poll(async () => (await rpc('state')).agents[0].threshold)
        .toBe(0.97);
    });
    await writeFile(configPath, 'unfinished json');
    await page.getByRole('button', {name: 'Revoke access'}).click();
    await page.getByRole('button', {name: 'Confirm revocation'}).click();
    await expect(
      page.getByText('Access revoked. Configuration cleanup is pending.'),
    ).toBeVisible();
    await expect(
      ask('beside_recall', {task: 'Check a preference.'}),
    ).rejects.toThrow('not connected');
    await expect(readFile(config.args.at(-1))).rejects.toMatchObject({
      code: 'ENOENT',
    });
    expect(await readFile(configPath, 'utf8')).toBe('unfinished json');
    await writeFile(configPath, installed);
    await page.getByRole('button', {name: 'Finish removal'}).click();
    await expect(
      page.getByRole('button', {name: '+ Claude Code'}),
    ).toBeVisible();
    expect(JSON.parse(await readFile(configPath, 'utf8'))).toEqual(personal);
  } finally {
    agent?.stdin?.end();
    core.stdin.end();
  }
});

test('three weeks of observed choices lead to one user-approved revision', async ({
  page,
}) => {
  const text = 'I decline all recurring meetings.';
  const rule = {
    id: 'recurring',
    section: 'deciding',
    authored: true,
    confidence: 1,
    evidence: [],
    sources: [],
  };
  const revision = createHash('sha256').update(text).digest('hex');
  const markdown =
    '# How you decide\n\n## Recurring meetings\n' +
    text +
    '\n<!-- beside ' +
    JSON.stringify(rule) +
    ' -->\n';
  const scores = [21, 10, 1].map((days, index) => ({
    id: 'case-' + index,
    date: new Date(Date.now() - days * 86400000).toISOString(),
    category: 'decisions',
    scope: 'live',
    grade: 0,
    question: [
      'Join a monthly design review?',
      'Attend the weekly critique?',
      'Accept the recurring studio invite?',
    ][index],
    guess: 'I would decline.',
    actual: [
      'Join the monthly review.',
      'Attend the weekly critique.',
      'Accept the studio invite.',
    ][index],
    ruleIds: ['recurring'],
    ruleRevisions: {recurring: revision},
    ruleCheck: {id: 'recurring', revision},
  }));
  const {rpc, core, folder} = await fixtureNotebook(false, {markdown, scores});
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Memory', exact: true}).click();
    const reading = page.getByText('How well Beside guessed', {exact: true});
    await expect(reading).toBeVisible();
    await reading.click();
    await expect(page.getByText(/Beside hid \d+ replies/)).toHaveCount(0);
    await page.getByRole('button', {name: 'Read the predictions'}).click();
    await expect(page.getByText(/possible exceptions on 3 days/)).toBeVisible();
    await page.getByText('Read the 3 observations').click();
    await expect(
      page
        .locator('blockquote')
        .filter({hasText: 'Attend the weekly critique.'}),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Change or exception?'}).click();
    const revised = 'I make time for recurring studio critiques.';
    await page
      .getByLabel('How would you word the change or exception?')
      .fill(revised);
    expect(await readFile(join(folder, 'scroll/deciding.md'), 'utf8')).toContain(text);
    await page.getByRole('button', {name: 'A change', exact: true}).click();
    await expect(page.getByRole('heading', {name: 'Surprises'})).toHaveCount(0);
    await page.getByRole('button', {name: '← Memory'}).click();
    await expect(page.getByRole('button', {name: revised})).toBeVisible();
    expect(
      await readFile(join(folder, 'scroll/deciding.md'), 'utf8'),
    ).toContain(revised);
    const state = await rpc('state');
    expect(state.scores.map((v: any) => v.grade)).toEqual([0, 0, 0]);
    expect(
      state.scores.every((v: any) => v.reflection === revised),
    ).toBeTruthy();
  } finally {
    core.stdin.end();
  }
});

test('Desk matches approved goals to the active paragraph and remembers a dismissal', async ({
  page,
}) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    const first = await rpc('create', {
      section: 'goals',
      text: 'Read about studio lighting before buying equipment.',
    });
    const strongest = 'Practice studio lighting with portraits.';
    const id = 'a'.repeat(32);
    const goals = join(folder, 'scroll/goals.md');
    await writeFile(
      goals,
      (await readFile(goals, 'utf8')) +
        '\n## Portraits\n' +
        strongest +
        '\n<!-- beside ' +
        JSON.stringify({
          id,
          status: 'stamped',
          confidence: 0.95,
          importance: 1,
          updated: new Date().toISOString(),
        }) +
        ' -->\n',
    );
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.clock.install();
    await page.goto('/');
    const chat = page.getByRole('navigation', {name: 'Workspace sections'}).getByRole('button', {name: 'Chat'});
    await chat.click();
    const sheet = page.getByLabel('Desk sheet');
    const note = page.getByRole('complementary', {
      name: 'Goal from your Scroll',
    });
    await sheet.fill('Think about next week before bed.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    await sheet.fill('Studios and lightings are plurals.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    const related = 'Adjust studio lighting for portraits.';
    await sheet.fill(related + '\nA grocery list for Sunday.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    await sheet.press('Meta+ArrowUp');
    await expect(note).toContainText(strongest);
    await note.getByRole('button', {name: 'Dismiss margin note'}).click();
    await expect(
      page.getByText('Margin notes quiet for 24 hours.'),
    ).toBeVisible();
    await page.reload();
    await chat.click();
    await sheet.fill('Read about studio lighting before buying equipment.');
    await page.clock.runFor(900);
    await expect(note).toHaveCount(0);
    const muted = (await rpc('state')).marginMuted;
    await page.clock.setFixedTime(
      new Date(Date.parse(muted.quietUntil) + 1000),
    );
    await sheet.fill('Adjust studio lighting before buying equipment.');
    await expect(note).toContainText(first.text);
    await sheet.fill(related);
    await expect(note).toContainText(first.text);
  } finally {
    core.stdin.end();
  }
});

test('resume the publisher’s real GGUF download and seal a decision without an external runtime', async ({
  page,
}) => {
  test.skip(
    process.env.BESIDE_TEST_MODEL_DOWNLOAD !== '1',
    'Requires the hardware-tier GGUF download.',
  );
  test.setTimeout(1800000);
  page.setDefaultTimeout(15000);
  const first = await fixtureNotebook();
  const partial = join(
    first.folder,
    '.beside',
    'models',
    (await first.rpc('modelDownloadStatus')).file + '.partial',
  );
  let writer: typeof first | undefined, reader: typeof first | undefined;
  const closed = (core: typeof first.core) =>
    new Promise<void>(resolve => {
      if (core.exitCode !== null || core.signalCode !== null) resolve();
      else core.once('exit', () => resolve());
    });
  try {
    await first.rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    // A verified local copy can seed the prefix on repeat runs. Both continuations
    // still use the publisher, and the runtime verifies the entire finished file.
    let startingBytes = 0;
    const seed = process.env.BESIDE_TEST_GGUF_PREFIX;
    if (seed && basename(seed) === basename(partial, '.partial')) {
      const bytes = (await stat(seed)).size;
      startingBytes = Math.max(0, bytes - 96 * 1024 * 1024);
      await mkdir(dirname(partial), {recursive: true});
      await copyFile(seed, partial, constants.COPYFILE_FICLONE);
      await truncate(partial, startingBytes);
    }
    first.core.stdin.write(
      JSON.stringify({operation: 'downloadBundled', args: {}}) + '\n',
    );
    await expect
      .poll(async () => (await stat(partial).catch(() => null))?.size ?? 0, {
        timeout: 120000,
        intervals: [100],
      })
      .toBeGreaterThan(startingBytes + 4_000_000);
    first.core.kill('SIGTERM');
    await closed(first.core);
    const offset = (await stat(partial)).size;
    writer = await fixtureNotebook(false, undefined, first.folder);
    reader = await fixtureNotebook(false, undefined, first.folder);
    const steps: number[] = [];
    await page.exposeFunction('besideTestIPC', async (op: string, args: any) => {
      if (op === 'models') return []; // Model a Mac with no exposed Ollama or LM Studio.
      if (op === 'modelDownloadStatus') {
        const progress = await reader!.rpc(op, args);
        steps.push(progress.bytes);
        return progress;
      }
      try {
        return await writer!.rpc(op, args);
      } catch (e) {
        if (op === 'downloadBundled') console.error('Bundled download:', e);
        throw e;
      }
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(
      page.getByRole('heading', {name: 'Choose how Beside answers'}),
    ).toBeVisible();
    await page
      .getByRole('button', {name: 'Use a model on this Mac', exact: true})
      .click();
    await page.getByRole('button', {name: /^Resume .* download$/}).click();
    await expect(
      page.getByRole('progressbar', {name: 'Local model download'}),
    ).toBeVisible();
    await expect(page.getByText('Ready to think on this Mac.')).toBeVisible({
      timeout: 1500000,
    });
    expect(steps).toContain(offset);
    expect(steps.some(n => n > offset)).toBeTruthy();
    await page.getByRole('button', {name: 'Continue', exact: true}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.reload();
    await page
      .getByLabel('Desk sheet')
      .fill('Should I sketch the cover first or start painting right away?');
    await page.getByRole('button', {name: 'Seal a guess'}).click();
    await page.getByRole('button', {name: 'Open sealed guess'}).click();
    await page
      .getByLabel('Your decision')
      .fill('I will sketch the cover first.');
    await page.getByRole('button', {name: 'Break the seal'}).click();
    const state = await writer.rpc('state');
    expect(state.model.runtime).toBe('Beside');
    expect(state.sealed.guess.trim().length).toBeGreaterThan(0);
    console.log(
      'Real bundled download resumed at',
      offset,
      'bytes. Model:',
      state.model.name,
    );
  } finally {
    for (const session of [first, reader, writer]) {
      if (!session) continue;
      if (session.core.exitCode === null && session.core.signalCode === null)
        session.core.stdin.end();
      await closed(session.core);
    }
    await rm(first.folder, {recursive: true, force: true});
  }
});

test('Settings asks for consent and finishes ChatGPT sign-in before switching to cloud', async ({
  page,
}, info) => {
  const {rpc, core} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.1, 0.56],
        [0.18, 0.43],
        [0.28, 0.35],
        [0.39, 0.36],
        [0.52, 0.46],
        [0.67, 0.59],
        [0.78, 0.6],
        [0.9, 0.48],
      ],
    });
    let attempts = 0;
    let finishCheck: (() => void) | undefined;
    await page.exposeFunction('besideTestIPC', async (op: string, args: any) => {
      // Browser automation cannot consent on a person's OpenAI account.
      // The native transport test checks the actual HTTP and streaming contract.
      if (op === 'connectPlan') {
        if (++attempts === 1) throw Error('Sign-in was cancelled.');
        await new Promise<void>(resolve => {
          finishCheck = resolve;
        });
        return rpc('fixturePlanAccount');
      }
      if (op === 'brainConfigure') {
        expect(args).toMatchObject({mode: 'cloud', provider: 'ChatGPT', cloudConsent: true});
      }
      return rpc(op, args);
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/#settings');
    await page.getByRole('button', {name: 'Set up later'}).click();
    await expect(page.getByRole('button', {name: 'Settings', exact: true})).toHaveAttribute('aria-current', 'page');
    await page.getByRole('tab', {name: 'Cloud', exact: true}).click();
    const signIn = page.getByRole('button', {name: 'Sign in with ChatGPT'});
    await expect(signIn).toBeDisabled();
    await page.getByRole('checkbox', {name: /Allow screen & memory context/}).check();
    await signIn.click();
    await expect(page.getByRole('alert')).toContainText('cancelled');
    expect((await rpc('brainStatus')).mode).toBe('local');
    await signIn.click();
    await expect(page.getByRole('button', {name: 'Waiting for sign-in…'})).toBeDisabled();
    finishCheck!();
    await expect(page.getByText('Connected to ChatGPT.', {exact: true})).toBeVisible();
    await expect(page.getByRole('button', {name: 'Disconnect', exact: true})).toBeVisible();
    expect((await rpc('state')).brain).toMatchObject({mode: 'cloud', provider: 'ChatGPT', cloudConsent: true});
  } finally {
    core.stdin.end();
  }
});

test('one sheet keeps a conversation, a correction, and a reusable method', async ({
  page,
}, info) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.14, 0.65],
        [0.48, 0.23],
        [0.7, 0.71],
        [0.9, 0.47],
      ],
    });
    await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await rpc('create', {
      section: 'deciding',
      text: 'For museum visits, I pick one painting to spend time with.',
    });
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name:'Chat', exact:true}).click();
    const field = page.getByLabel('Desk sheet');
    async function ask(text: string, count: number) {
      await field.fill(text);
      await page.getByRole('button', {name: 'Ask Beside', exact: true}).click();
      await expect(page.locator('.exchange')).toHaveCount(count, {
        timeout: 90000,
      });
      await expect(field).toHaveValue('');
    }
    await ask(
      'Help me plan a museum visit. I have twenty minutes to see a painting called Blue Heron. Give me two short steps.',
      1,
    );
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText('For museum visits');
    await page.screenshot({path: info.outputPath('conversation.png')});
    const first = await rpc('state');
    expect(
      await readFile(
        join(folder, 'desk', 'conversations', first.current + '.md'),
        'utf8',
      ),
    ).toContain(first.chat.turns[0].reply);
    await page.reload();
    await page.getByRole('button', {name:'Chat', exact:true}).click();
    await ask('Remind me of the name of the painting I mentioned.', 2);
    expect((await rpc('state')).chat.turns[1].reply).toMatch(/Blue Heron/i);
    await ask(
      'Help me plan a museum visit again, this time with five minutes to look at the painting.',
      3,
    );
    await ask(
      'For every museum visit, always choose one painting and give me a question to think about.',
      4,
    );
    const skill = (await rpc('state')).skills[0];
    expect(skill.title.trim()).toBeTruthy();
    expect(
      await readFile(join(folder, 'scroll', 'skills', skill.file), 'utf8'),
    ).toContain('one painting');
    await page.getByRole('button', {name: 'Memory', exact: true}).click();
    await page.getByText(skill.title, {exact: true}).click();
    await expect(page.getByText(skill.body, {exact: true})).toBeVisible();
    await page.screenshot({path: info.outputPath('scroll-skills.png')});
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: /New sheet/}).click();
    await expect(page.locator('.exchange')).toHaveCount(0);
    await ask('Give me a plan for a museum visit tomorrow.', 1);
    const next = await rpc('state');
    expect(next.current).not.toBe(first.current);
    expect(next.chat.turns[0].skill).toBe(skill.title);
    expect(next.chat.private).toBe(true);
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText(
      'Skill · ' + skill.title,
    );
    await page.emulateMedia({colorScheme: 'dark'});
    await page.screenshot({path: info.outputPath('night-ink.png')});
    await page.setViewportSize({width: 520, height: 600});
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBeLessThanOrEqual(520);
    await page.getByRole('button', {name: 'Correct this reply'}).click();
    await expect(page.getByText('The reply you’re correcting')).toBeVisible();
    const state = await rpc('state');
    const target = {sheet: state.current, turn: state.chat.turns.at(-1).id};
    await page.reload();
    await expect(page.locator('.quoted-reply')).toContainText(state.chat.turns.at(-1).reply.slice(0, 100));
    await expect(page.getByLabel('Your correction')).toBeVisible();
    await page.getByLabel('Your correction').fill('For a short museum visit, ask me about the painting’s colors first.');
    await page.getByRole('button', {name: 'Read my correction'}).click();
    await expect(page.getByLabel('What Beside learned')).toBeVisible({timeout: 90000});
    expect((await rpc('state')).firstCorrection.context).toEqual(target);
  } finally {
    core.stdin.end();
  }
});

test('a dinner question chooses its own tools and remembers a change of mind', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.12, 0.6],
        [0.43, 0.25],
        [0.63, 0.72],
        [0.88, 0.4],
      ],
    });
    await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    const field = page.getByLabel('Desk sheet');
    async function say(text: string) {
      await field.fill(text);
      await field.press('Enter');
      await expect(page.locator('.exchange')).toHaveCount(1, {timeout: 90000});
      await expect(field).toHaveValue('');
    }
    async function fresh() {
      await page.getByRole('button', {name: /^Sheets/}).click();
      await page.getByRole('button', {name: /New sheet/}).click();
      await expect(page.locator('.exchange')).toHaveCount(0);
    }
    const before =
      'When we split a dinner bill, always give me just the amount per person.';
    const after =
      'For dinner bills, from now on show the calculation as well as the amount per person.';
    await say(before);
    const first = (await rpc('state')).skills[0];
    expect(first.title.trim()).toBeTruthy();
    expect(first.body).toBe(before);
    await fresh();
    await say(after);
    const updated = (await rpc('state')).skills;
    expect(updated).toHaveLength(1);
    expect(updated[0].id).toBe(first.id);
    expect(updated[0].body).toBe(after);
    await page.reload();
    await fresh();
    await say(
      'Can you split a $96 dinner bill between four people and tell me what day tomorrow is?',
    );
    const turn = (await rpc('state')).chat.turns[0];
    const tomorrow = new Date(turn.at);
    tomorrow.setDate(tomorrow.getDate() + 1);
    const weekday = tomorrow.toLocaleDateString('en-US', {weekday: 'long'});
    const expected = tomorrow.toLocaleDateString('en-CA');
    expect(turn.tools.find((v: any) => v.tool === 'Calendar').date).toBe(
      expected,
    );
    expect(turn.tools.find((v: any) => v.tool === 'Calculator').value).toBe(24);
    expect(turn.skill).toBe(updated[0].title);
    await expect(page.locator('.reply')).toContainText('24');
    await expect(page.locator('.reply')).toContainText(weekday);
    await expect(page.getByLabel('Working')).toContainText('96/4 = 24');
    await page.screenshot({path: info.outputPath('dinner.png')});
    await page.getByText('Behind this reply', {exact: true}).click();
    await expect(page.locator('.receipt')).toContainText(updated[0].title);
    await fresh();
    await say(
      'What did I originally ask you to do with dinner bills, and how did that change?',
    );
    const timeline = page.getByLabel('Your earlier instructions');
    await expect(timeline.locator('li')).toHaveCount(2);
    await expect(timeline.locator('li').first()).toContainText(before);
    await expect(timeline.locator('li').last()).toContainText(after);
    await page.screenshot({path: info.outputPath('timeline.png')});
    await fresh();
    await say('My pottery budget is 84 dollars.');
    await fresh();
    await say(
      'Using the pottery budget I mentioned, how much can I spend per class if I take four classes?',
    );
    const budget = (await rpc('state')).chat.turns[0];
    const passages = budget.tools.find(
      (v: any) => v.tool === 'Notebook',
    ).passages;
    expect(passages.map((v: any) => v.words)).toEqual([
      'My pottery budget is 84 dollars.',
    ]);
    expect(budget.tools.find((v: any) => v.tool === 'Calculator')?.value).toBe(
      21,
    );
    await expect(page.locator('.reply')).toContainText('21');
  } finally {
    core.stdin.end();
  }
});

test('a public question in the floating sheet shows the exact web source in place', async ({page}) => {
  test.skip(process.env.BESIDE_LIVE_SEARCH !== '1', 'Uses live public search and an installed model.');
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    await rpc('fixtureActivity', {
      title: 'Notes · Staff meeting', source: 'display',
      body: 'The private rehearsal has moved to the north courtyard.',
    });
    let outbound = '';
    let result: Row = {};
    await page.exposeFunction('publicNotebook', async (op: string, args: Row) => {
      if (op === 'activitySize') return null;
      if (op === 'visitPublicSource') { outbound = args.url; return null; }
      const value = await rpc(op, args);
      if (op === 'activityAsk') result = value;
      return value;
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: Row) => cmd === 'notebook'
          ? (window as any).publicNotebook(p.operation, p.args) : Promise.resolve(1),
      };
    });
    await page.goto('/#activity');
    const question = 'What do the official docs say is the shortcut for applying checklist format in Apple Notes on Mac?';
    const field = page.getByRole('textbox', {name: 'Your question'});
    await field.fill(question);
    await field.press('Enter');
    await expect(page.locator('.activity-turn .reply')).toContainText(/Shift.{0,12}(Command|Cmd).{0,8}L|⇧\s*⌘\s*L/i, {timeout: 90000});
    const apple = result.webSources.find((p: Row) => new URL(p.url).hostname === 'support.apple.com' && /checklist.{0,20}shift/i.test(p.quote));
    expect(apple).toBeTruthy();
    expect(result.publicQuery).toBe(question);
    expect(result.sources).toEqual([]);
    expect(result.notes).toEqual([]);
    expect(result.pointLine).toBe('');
    expect(result.usedVisual).toBeFalsy();
    const sources = page.getByRole('group', {name: 'Sources used'});
    await sources.getByRole('button', {name: apple.title, exact: true}).click();
    const passage = page.getByRole('region', {name: apple.title});
    await expect(passage).toContainText(apple.quote);
    expect(outbound).toBe('');
    await passage.getByRole('button', {name: 'Open source'}).click();
    await expect.poll(() => outbound).toBe(apple.url);

    const followup = page.getByRole('textbox', {name: /^(Your question|Follow up)$/});
    await followup.fill('Look this up');
    await followup.press('Enter');
    await expect(page.locator('.activity-turn .reply').last()).toContainText('What public topic');
    await followup.fill('Apple Notes checklist shortcut for Mac');
    await followup.press('Enter');
    await expect.poll(() => result.publicQuery, {timeout: 90000}).toBe('Apple Notes checklist shortcut for Mac');
    expect(result.webSources.length).toBeGreaterThan(0);
    expect(result.sources).toEqual([]);
    expect(result.pointLine).toBe('');

    await followup.fill('What should I do this weekend in my area?');
    await followup.press('Enter');
    await expect(page.locator('.activity-turn .reply').last()).toContainText('Which city');
    expect(result.publicQuery).toBeUndefined();
    await followup.fill('Boston');
    await followup.press('Enter');
    await expect.poll(() => result.publicQuery, {timeout: 90000}).toContain('Boston');
    expect(result.publicQuery).not.toMatch(/rehearsal|courtyard|my area/i);
    await expect(page.locator('.activity-turn .reply').last()).toContainText(/Saturday|Sunday|couldn’t confirm|couldn’t verify/);
  } finally {
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});

test('a weekend plan uses live sources and the current preference from another sheet', async ({
  page,
}, info) => {
  test.skip(
    process.env.BESIDE_LIVE_SEARCH !== '1',
    'Uses the free public search service and an installed local model.',
  );
  // Several conversations share this run. Each answer still has its own 90-second deadline.
  if (process.env.BESIDE_TEST_INKLING === '1') test.setTimeout(300_000);
  const {core, rpc} = await fixtureNotebook();
  try {
    await rpc('mark', {
      points: [
        [0.12, 0.75],
        [0.48, 0.35],
        [0.85, 0.52],
      ],
    });
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {name: 'llama3.2:3b', runtime: 'Ollama'});
    await rpc('finish');
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideTestIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    const field = page.getByLabel('Desk sheet');
    async function say(text: string) {
      await field.fill(text);
      await field.press('Enter');
      await expect(field).toHaveValue('', {timeout: 90000});
    }
    await say(
      'I live in Boston. On weekends I love hands-on printmaking workshops.',
    );
    await page.getByRole('button', {name: /^Sheets/}).click();
    await page.getByRole('button', {name: /New sheet/}).click();
    await say('What should I do this weekend?');
    await expect(page.locator('.source-link').first()).toBeVisible({
      timeout: 90000,
    });
    const result = (await rpc('state')).chat.turns[0];
    expect(result.web.query).toContain('Boston');
    expect(result.web.query).toContain('art workshops');
    expect(result.web.status).toBe('done');
    expect(result.web.sources[result.web.pick].url).toMatch(/^https?:\/\//);
    expect(result.web.quote).toBeTruthy();
    expect(result.web.dates.some((iso: string) => {
      const date = new Date(iso + 'T12:00:00');
      return result.reply.includes(date.toLocaleDateString('en-US', {month: 'long', day: 'numeric'}));
    })).toBe(true);
    expect(result.reply.length).toBeGreaterThan(30);
    await page.getByLabel('Show remembered preference').hover();
    await expect(page.getByLabel('Remembered words')).toContainText(
      'printmaking',
    );
    await page.screenshot({path: info.outputPath('weekend.png')});
    await page.getByRole('button', {name: 'Not true anymore'}).click();
    await page.reload();
    await expect(page.locator('.source-link').first()).toBeVisible();
    await page.keyboard.press('Meta+k');
    await page.getByText('Earlier memories', {exact: true}).click();
    await expect(page.locator('.dated-notes')).toContainText('printmaking');
    await page.keyboard.press('Meta+,');
    await page.getByText('Web search and imports', {exact: true}).click();
    await page.getByLabel('Automatically look up local listings').check();
    await page.keyboard.press('Meta+3');
    async function fresh() {
      await page.getByRole('button', {name: /^Sheets/}).click();
      await page.getByRole('button', {name: /New sheet/}).click();
    }
    await fresh();
    await say('What should I do this weekend?');
    const automatic = (await rpc('state')).chat.turns[0];
    expect(automatic.web.status).toBe('done');
    expect(automatic.web.query).not.toMatch(/printmaking|art workshops/i);
    expect(automatic.usedMemory).toBeUndefined();
    await fresh();
    await say('What do the official docs say is the shortcut for applying checklist format in Apple Notes on Mac?');
    const lookup = (await rpc('state')).chat.turns[0];
    expect(lookup.web.sources.some((source: Row) => new URL(source.url).hostname === 'support.apple.com')).toBe(true);
    expect(lookup.selector).toBeNull();
    expect(lookup.notes).toEqual([]);
    await fresh();
    await page.getByLabel('Keep this sheet offline').check();
    await say('What should I do this weekend?');
    const offline = (await rpc('state')).chat.turns[0];
    expect(offline.web.status).toBe('private');
    expect(offline.web.sources).toBeUndefined();
    await expect(page.locator('.source-list, .search-proposal')).toHaveCount(0);
    console.log(
      'Weekend recommendation:',
      result.reply,
      'Source:',
      result.web.sources[result.web.pick].url,
    );
  } finally {
    core.stdin.end();
  }
});

test('read a workshop schedule, follow up from the file, then correct a dated project memory', async ({
  page,
}, info) => {
  let chunks = 0;
  const {rpc, core} = await fixtureNotebook(false, undefined, undefined, m => {
    chunks++;
    void page
      .evaluate(message => (window as any).besideDelta?.(message.payload), m)
      .catch(() => {});
  });
  try {
    await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('mark', {
      points: [
        [0, 0],
        [1, 1],
      ],
    });
    await rpc('finish');
    await page.exposeFunction('besideTestIPC', rpc);
    await page.addInitScript(() => {
      const callbacks = new Map();
      let serial = 0;
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: (fn: any) => {
          callbacks.set(++serial, fn);
          return serial;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        invoke: (cmd: string, p: any) => {
          if (cmd === 'notebook')
            return (window as any).besideTestIPC(p.operation, p.args);
          if (cmd === 'plugin:event|listen') {
            const emit = (payload?: unknown) =>
              callbacks.get(p.handler)?.({event: p.event, payload, id: p.handler});
            if (p.event === 'reply-delta') (window as any).besideDelta = emit;
            if (p.event === 'fresh-sheet') (window as any).freshDeskShortcut = emit;
          }
          return Promise.resolve(1);
        },
      };
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => {},
      };
    });
    await page.goto('/');
    await page.getByRole('button', {name: 'Chat', exact: true}).click();
    const field = page.getByLabel('Desk sheet');
    await page.getByLabel('Choose attachments').setInputFiles({
      name: 'studio-classes.csv',
      mimeType: 'text/csv',
      buffer: Buffer.from(
        'Workshop,Day,Start,Seats\nLinocut,Saturday,10 am,8\nBookbinding,Sunday,2 pm,6',
      ),
    });
    await expect(
      page.getByRole('button', {name: 'Remove studio-classes.csv'}),
    ).toBeVisible();
    await field.fill('Which workshop in my file is on Sunday?');
    await field.press('Enter');
    await expect(
      page.locator('.exchange').last().locator('.reply'),
    ).toContainText(/bookbinding/i, {timeout: 90000});
    expect(chunks).toBeGreaterThan(1);
    await page.getByRole('button', {name: 'Maybe later'}).click();
    await field.fill('How many seats does that Sunday class have?');
    await field.press('Enter');
    await expect(page.locator('.exchange')).toHaveCount(2, {timeout: 90000});
    await expect(
      page.locator('.exchange').last().locator('.reply'),
    ).toContainText(/6|six/i);
    await page.screenshot({path: info.outputPath('workshop.png')});
    await page.locator('.receipt summary').last().click();
    await page.getByRole('button', {name: 'Try this answer again'}).click();
    await expect(
      page.getByRole('button', {name: 'Try this answer again'}),
    ).toBeEnabled({timeout: 90000});
    expect((await rpc('state')).chat.turns[1].regenerations).toBe(1);
    await field.fill('I am working on the spring exhibition.');
    await field.press('Enter');
    await expect(page.locator('.exchange')).toHaveCount(3, {timeout: 90000});
    await page.keyboard.press('Meta+k');
    await page
      .getByText('I am working on the spring exhibition', {exact: true})
      .click();
    await page.getByRole('button', {name: 'Edit', exact: true}).click();
    await page
      .getByLabel('Correct this memory')
      .fill('I am working on the autumn exhibition.');
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByText('I am working on the autumn exhibition.', {exact: true}),
    ).toBeVisible();
    await page.reload();
    await expect(page.getByRole('button', {name: 'Memory', exact: true})).toHaveAttribute('aria-current', 'page');
    await expect(
      page.getByText('I am working on the autumn exhibition.', {exact: true}),
    ).toBeVisible();
    // The macOS global shortcut delivers this event to the notebook webview.
    const previousSheet = (await rpc('state')).current;
    await page.evaluate(() => (window as any).freshDeskShortcut());
    await expect(field).toBeVisible();
    await expect.poll(async () => (await rpc('state')).current).not.toBe(previousSheet);
    await expect(page.locator('.exchange')).toHaveCount(0);
    await field.fill(
      'What did you say about bookbinding in our previous chat?',
    );
    await field.press('Enter');
    const recalled = page.locator('.exchange').last();
    await expect(recalled.locator('.reply')).toContainText(/bookbinding/i, {
      timeout: 90000,
    });
    await recalled.locator('.receipt summary').click();
    const passage = recalled
      .locator('.receipt blockquote')
      .filter({hasText: /bookbinding/i})
      .first();
    await expect(passage).toContainText('Assistant ·');
    await page.keyboard.press('Meta+,');
    await page.getByText('Your notebook', {exact: true}).click();
    await page.getByRole('button', {name: 'Review local measures'}).click();
    await expect(
      page.getByRole('columnheader', {name: 'Rejected'}),
    ).toBeVisible();
    const currentReport = page.getByRole('table', {
      name: /Seven-day periods through/,
    });
    await expect(currentReport.getByRole('row').last()).toContainText(
      /of [1-9]\d* \(\d+(?:[.,]\d+)?%\)/,
    );
    await currentReport.screenshot({path: info.outputPath('two-weeks.png')});
    await page.getByLabel('Two weeks ending').fill('2024-03-12');
    await page.getByRole('button', {name: 'Apply', exact: true}).click();
    const report = page.getByRole('table', {
      name: 'Seven-day periods through 2024-03-12',
    });
    await expect(report.getByRole('row')).toHaveCount(3);
    await expect(report.getByRole('cell', {name: '2024-02-28'})).toBeVisible();
    await expect(report.getByRole('cell', {name: '2024-03-06'})).toBeVisible();
    await expect(report.getByRole('cell', {name: 'No replies'})).toHaveCount(2);
    const download = await rpc('exportNotebook');
    try {
      expect(
        execFileSync('unzip', ['-p', download, 'records/note.json'], {
          encoding: 'utf8',
        }),
      ).toContain('autumn exhibition');
      expect(
        execFileSync('unzip', ['-Z1', download], {encoding: 'utf8'}),
      ).not.toContain('.beside/');
    } finally {
      await rm(download);
    }
  } finally {
    core.stdin.end();
  }
});

test('ask anywhere remembers a window, credits a source, learns a correction and forgets it', async ({
  browser,
}, info) => {
  test.setTimeout(process.env.BESIDE_TEST_INKLING === '1' ? 300000 : 150000);
  const {core, rpc, folder} = await fixtureNotebook();
  const context = await browser.newContext({
    viewport: {width: 640, height: 540},
    recordVideo: {dir: info.outputDir, size: {width: 640, height: 540}},
  });
  try {
    const page = await context.newPage();
    page.setDefaultTimeout(12000);
    await page.exposeFunction('besideActivityIPC', (op: string, args: object) =>
      op === 'activitySize' ? null : rpc(op, args),
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).besideActivityIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await page.goto('/#activity');
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await page.getByRole('button', {name: 'Sources', exact: true}).click();
    await page.getByLabel('Messages', {exact: true}).check();
    await page.getByLabel('Website hostname').fill('x.com');
    await page.getByRole('button', {name: 'Allow site'}).click();
    await page.getByRole('button', {name: 'Start', exact: true}).click();
    await page.getByText('Visual screenshots & vision', {exact: true}).click();
    await page.getByLabel('Save visual screen previews (24h, encrypted)').check();
    await rpc('fixtureActivity', {
      source: 'site:x.com',
      title: 'Studio announcements',
      body: 'A poster for the Shoreline exhibition. Sunday at West Pier Gallery.',
    });
    const sample = await context.newPage();
    await sample.setContent(
      '<main style="font:22px Georgia; padding:32px;color:#262320;background:#fcfaf5;height:100vh"><h2>Messages · Rowan</h2><p>North Window exhibition</p><p>Let’s see the North Window exhibition together on Saturday morning.</p><div style="background:#a23b37;width:105px;height:60px;margin:16px"></div></main>',
    );
    const picture = await sample.screenshot();
    await sample.close();
    await rpc('fixtureActivity', {
      source: 'com.apple.MobileSMS',
      title: 'Conversation with Rowan',
      fixtureImage: picture.toString('base64'),
    });
    await rpc('mark', {
      points: [
        [0.2, 0.3],
        [0.6, 0.65],
      ],
    });
    await rpc('finish');
    await page.goto('/');
    await page.reload();
    await page.getByRole('navigation', {name: 'Workspace sections'}).getByRole('button', {name: 'Chat'}).click();
    await page
      .getByLabel('Desk sheet')
      .fill('Which exhibition did I discuss with Rowan?');
    await page.getByRole('button', {name: 'Ask Beside', exact: true}).click();
    await expect(page.locator('.activity-turn').first()).toContainText(
      /North Window/i,
      {timeout: 120000},
    );
    await page
      .getByRole('button', {name: /\[\d+\] Messages/})
      .first()
      .click();
    await page.getByText(/^Extracted text \(\d+ lines\)/).click();
    await expect(page.locator('.evidence-raw-body')).toContainText(
      /Saturday morning/i,
    );
    await page.screenshot({path: info.outputPath('local-evidence.png')});
    await page.getByRole('button', {name: 'Correct', exact: true}).click();
    await page
      .getByLabel('What should Beside understand here?')
      .fill('For exhibitions with Rowan I prefer Sunday afternoon.');
    await page
      .getByRole('button', {name: 'Save correction', exact: true})
      .click();
    await page
      .getByLabel('Your question', {exact: true})
      .fill(
        'When should I go to the exhibition with Rowan, given my correction?',
      );
    await page.getByRole('button', {name: 'Ask on this Mac'}).click();
    await expect(page.locator('.activity-turn .reply').last()).toContainText(
      /Sunday/i,
      {timeout: 120000},
    );
    if (process.env.BESIDE_TEST_INKLING === '1') {
      const question = 'What color is the rectangle in the Messages screenshot?';
      await page.getByLabel('Follow up').fill(question);
      await page.getByRole('button', {name: 'Ask on this Mac'}).click();
      const visualTurn = page.locator('.activity-turn').filter({
        has: page.getByText(question, {exact: true}),
      });
      await expect(visualTurn.locator('.reply')).toContainText(
        /\b(?:red|crimson|burgundy|maroon|brick|rust)\b/i,
        {timeout: 120000},
      );
      await expect(
        page.getByText('Inkling used the screen image on this Mac.', {exact: true}),
      ).toBeVisible();
    }
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await expect(
      page.getByText('For exhibitions with Rowan I prefer Sunday afternoon.', {
        exact: true,
      }),
    ).toBeVisible();
    await page.getByRole('button', {name: 'Sources', exact: true}).click();
    await page.getByRole('button', {name: 'Pause', exact: true}).click();
    await page.getByText('Forget activity', {exact: true}).click();
    await page.getByRole('button', {name: 'Last hour', exact: true}).click();
    await page.getByRole('button', {name: 'Confirm', exact: true}).click();
    await expect(
      page.getByRole('status').filter({hasText: /forgotten/i}),
    ).toBeVisible();
    const journal = await rpc('activityJournal');
    expect(journal.events).toEqual([]);
    expect(journal.habits).toEqual([]);
    await page.getByRole('button', {name: 'Ask Beside', exact: true}).click();
    await page
      .getByLabel('Your question', {exact: true})
      .fill('Which exhibition did I discuss with Rowan?');
    await page.getByRole('button', {name: 'Ask on this Mac'}).click();
    await expect(page.getByText(/couldn’t find that/)).toBeVisible();
    expect((await rpc('state')).chat).toBeNull();
    const movie = page.video();
    await page.close();
    if (movie) await movie.saveAs(info.outputPath('ask-anywhere.webm'));
  } finally {
    await context.close();
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});

test('floating assistant reads an unopened local document and manages daily screen files', async ({
  page,
}, info) => {
  test.setTimeout(300000);
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    const docs = join(folder, 'example-documents');
    await mkdir(docs);
    await writeFile(
      join(docs, 'Studio schedule.md'),
      'The Juniper Lantern collection opens at seven on Saturday.',
    );
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await page.exposeFunction('assistantIPC', async (op: string, args: any) => {
      if (op === 'activitySettings')
        return {...(await rpc(op, args)), screenPermission: true};
      if (op === 'activityAddFolder') return rpc('fixtureDocumentFolder');
      if (op === 'activitySize') return null;
      return rpc(op, args);
    });
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: any) =>
          cmd === 'notebook'
            ? (window as any).assistantIPC(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/#activity');
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await page.getByRole('button', {name: 'Sources', exact: true}).click();
    await page.getByRole('button', {name: 'All displays'}).click();
    await page.getByRole('button', {name: 'Start', exact: true}).click();
    await rpc('fixtureActivity', {
      source: 'display',
      title: 'Screen 1 · Notes active',
      body: 'Window Help\nFile Edit View\nWindow Help\nWren press invitation\nThe Wren press opens Saturday at seven.\nWren press opening hours',
    });
    await page.getByText('Folders & local files', {exact: true}).click();
    await page.getByRole('button', {name: '+ Choose a folder…'}).click();
    const earlier = page.getByText('Earlier notes and mail', {exact: true});
    const importNotes = page.getByRole('button', {name: 'Import Apple Notes'});
    await expect(importNotes).toBeHidden();
    await earlier.click();
    await expect(importNotes).toBeVisible();
    await earlier.click();
    await page.getByLabel('Keep daily Markdown copies').check();
    const daily = join(
      folder,
      '.beside',
      'activity-journal',
      new Date().toISOString().slice(0, 10) + '.md',
    );
    await expect
      .poll(async () => readFile(daily, 'utf8').catch(() => ''))
      .toContain('Wren press');
    await page.getByRole('button', {name: 'Ask Beside', exact: true}).click();
    await page
      .getByLabel('Your question', {exact: true})
      .fill(
        'When does the Juniper Lantern collection open, according to the studio schedule?',
      );
    await page.getByRole('button', {name: 'Ask on this Mac'}).click();
    await expect(page.locator('.activity-turn .reply').last()).toContainText(
      /seven|\b7/i,
      {timeout: 150000},
    );
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await page.getByText('Last question', {exact: true}).click();
    await page
      .locator('.file-passage summary')
      .filter({hasText: 'Studio schedule.md'})
      .click();
    await expect(page.locator('.file-passage blockquote')).toContainText(
      'seven on Saturday',
    );
    await page.screenshot({path: info.outputPath('floating-file-answer.png')});
    await page.getByRole('button', {name: 'Ask Beside', exact: true}).click();
    await page.getByRole('button', {name: 'New', exact: true}).click();
    await page.getByLabel('Your question', {exact: true}).fill('What was I just looking at?');
    const recall = page.getByRole('button', {name: 'Ask on this Mac'});
    await recall.evaluate(button =>
      button.addEventListener('click', () => performance.mark('recall-click'), {
        once: true,
      }),
    );
    await page.evaluate(() => {
      const observer = new MutationObserver(() => {
        const quote = document.querySelector('.recall-preview blockquote');
        if (quote && quote.getBoundingClientRect().height > 0) {
          requestAnimationFrame(() =>
            requestAnimationFrame(() => performance.mark('passage-paint')),
          );
          observer.disconnect();
        }
      });
      observer.observe(document.body, {childList: true, subtree: true});
    });
    await recall.click();
    await expect(page.locator('.activity-turn .reply').last()).toContainText(
      /Wren press/i,
      {timeout: 120000},
    );
    await expect(page.locator('.activity-turn .recall-line')).toHaveText(
      /^(The Wren press opens Saturday at seven\.|Wren press invitation|Wren press opening hours)$/,
    );
    await expect(page.locator('.activity-turn .reply').last()).toContainText(
      'You were looking at',
    );
    const timing = await page.evaluate(() => {
      const start = performance.getEntriesByName('recall-click')[0].startTime;
      const paint = performance.getEntriesByName('passage-paint')[0]?.startTime;
      return {
        firstPaintMs: Math.round(paint - start),
        completedWithinMs: Math.round(performance.now() - start),
      };
    });
    console.log('Local screen recall timing (ms)', timing);
    expect(timing.firstPaintMs).toBeLessThan(1000);
    await expect(page.locator('.activity-turn .activity-source')).toHaveCount(
      0,
    );
    const menuLine = 'Menu: Carrot soup and sesame rolls.';
    await rpc('fixtureActivity', {
      source: 'display',
      title: 'Notes · Park lunch',
      body: 'Park lunch\n' + menuLine,
    });
    const input = page.getByRole('textbox', {name: 'Follow up'});
    await input.fill(
      'What was I planning to cook? Show the exact line in “Park lunch”.',
    );
    const painted = page.evaluate(
      () =>
        new Promise<number>(resolve => {
          document.addEventListener(
            'keydown',
            () => {
              const start = performance.now();
              const observe = () => {
                const turn = document.querySelector(
                  '.activity-turn:last-of-type',
                );
                if (
                  turn
                    ?.querySelector('.reply')
                    ?.textContent?.includes('Carrot soup') &&
                  turn.querySelector('blockquote')?.textContent ===
                    'Menu: Carrot soup and sesame rolls.'
                ) {
                  requestAnimationFrame(() =>
                    resolve(performance.now() - start),
                  );
                } else requestAnimationFrame(observe);
              };
              requestAnimationFrame(observe);
            },
            {once: true},
          );
        }),
    );
    await input.press('Enter');
    const menuMs = Math.round(await painted);
    console.log('Return to painted menu and exact quote (ms)', menuMs);
    expect(menuMs).toBeLessThan(1000);
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await page.getByRole('button', {name: 'Sources', exact: true}).click();
    await page.getByRole('button', {name: 'Pause', exact: true}).click();
    expect((await rpc('activitySettings')).enabled).toBe(false);
    await rpc('activityForget', {scope: 'all'});
    await expect(stat(daily)).rejects.toThrow();
    await page.getByRole('button', {name: 'Sources', exact: true}).click();
    await page.getByText('Folders & local files', {exact: true}).click();
    await page
      .getByRole('button', {name: /Remove .*example-documents/})
      .click();
    expect((await rpc('activitySettings')).folders).toHaveLength(0);
  } finally {
    core.stdin.end();
  }
});

test('a long floating reply keeps the next draft and its composer in view', async ({
  page,
}, info) => {
  let finish!: (value: any) => void;
  const answering = new Promise(resolve => {
    finish = resolve;
  });
  const packing = {id: 'packing-note', kind: 'note', title: 'Packing list', body: 'Bring the suitcase with wheels.'};
  await page.exposeFunction('sheetIPC', (op: string, args: Row) => {
    if (op === 'activitySettings') return {enabled: false, localModel: 'local'};
    if (op === 'activityAsk') return answering;
    if (op === 'activitySource' && args.id === packing.id) return packing;
    if (op === 'activityJournal') return {events: [], habits: [], remembered: []};
    if (op === 'activityEpoch') return 1;
    return null;
  });
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener() {},
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: (command: string, request: any) =>
        command === 'notebook'
          ? (window as any).sheetIPC(request.operation, request.args)
          : Promise.resolve(1),
    };
  });
  await page.setViewportSize({width: 512, height: 548});
  await page.emulateMedia({reducedMotion: 'reduce', colorScheme: 'dark'});
  await page.goto('/#activity');
  const composer = page.getByRole('textbox');
  await composer.fill('How should I prepare?');
  await composer.press('Enter');
  const status = page.getByRole('status');
  await expect(status).toHaveText('Thinking');
  const label = await status.locator('span').boundingBox();
  const signature = status.locator('svg');
  const glyph = await signature.boundingBox();
  expect(glyph!.x).toBeGreaterThanOrEqual(label!.x + label!.width);
  expect(glyph!.width).toBeLessThanOrEqual(18);
  expect((await status.boundingBox())!.height).toBeLessThanOrEqual(24);
  await expect(signature.locator('path')).toHaveCSS('animation-name', 'none');
  await page.emulateMedia({reducedMotion: 'no-preference', colorScheme: 'light'});
  await expect(signature.locator('path')).toHaveCSS('animation-name', 'sign-in-ink');
  await page.screenshot({path: info.outputPath('ink-status.png')});
  await composer.fill('Which step comes first?');
  const steps = Array.from(
    {length: 30},
    (_, i) => i + 1 + '. Check item ' + (i + 1),
  );
  finish({
    answer: ['## Preparation', ...steps].join('\n'),
    sources: [{id: 'fare', title: 'Baggage allowance'}, {id: packing.id, title: packing.title}],
    epoch: 1,
  });
  await expect(page.locator('.reply ol li')).toHaveCount(30);
  await expect(composer).toHaveValue('Which step comes first?');
  for (const width of [512, 390]) {
    await page.setViewportSize({width, height: 548});
    await page.locator('.reply li').last().scrollIntoViewIfNeeded();
    await expect(composer).toBeInViewport({ratio: 1});
    await expect(
      page.getByRole('button', {name: 'What Beside knows'}),
    ).toBeInViewport({
      ratio: 1,
    });
    expect(
      await page.evaluate(() => document.documentElement.scrollWidth),
    ).toBe(width);
  }
  const sourceLink = page.getByRole('button', {name: packing.title, exact: true});
  await expect(sourceLink).toBeHidden();
  await page.getByRole('button', {name: 'Baggage allowance and 1 more'}).click();
  await sourceLink.click();
  await expect(page.getByText(packing.body, {exact: true}).first()).toBeVisible();
});

test('the floating sheet keeps selected text local and offers a visible recording lifecycle', async ({
  page,
}) => {
  const calls: {op: string; args: any}[] = [];
  let selection = 'A wall surrounded Pataliputra, Chandragupta Maurya’s capital.';
  let phase = 'idle',
    latest = '';
  await page.exposeFunction('sheetIPC', (op: string, args: any) => {
    calls.push({op, args});
    if (op === 'activitySettings')
      return {
        enabled: true,
        scope: 'displays',
        sources: [],
        screenPermission: true,
        localModel: 'Inkling',
        lastObservedAt: Date.now() / 1000,
        questionProviders: ['ChatGPT'],
      };
    if (op === 'activityJournal') return {events:[],habits:[],remembered:[]};
    if (op === 'brainStatus') return {mode: 'local', availableCloud: []};
    if (op === 'models') return [];
    if (op === 'takeSelection') {
      const v = selection;
      selection = '';
      return v;
    }
    if (op === 'meetingStart') phase = 'recording';
    if (op === 'meetingStop') {
      phase = 'idle';
      latest = 'review.mp4';
    }
    if (op.startsWith('meeting') && op !== 'meetingNotes')
      return {phase, latest};
    if (op === 'meetingNotes')
      return {answer: 'Review the changes by Friday.', sources: [], epoch: 1};
    if (op === 'typedQuestion')
      return {
        answer: 'A public answer.',
        provider: 'ChatGPT',
        sources: [],
        epoch: 1,
      };
    if (op === 'activityAsk')
      return {
        answer: 'I can review those by Friday.',
        selection: args.selection,
        sources: [],
        epoch: 1,
      };
    if (op === 'activityEpoch') return 1;
    return null;
  });
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener() {},
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: (cmd: string, req: any) =>
        cmd === 'notebook'
          ? (window as any).sheetIPC(req.operation, req.args)
          : Promise.resolve(1),
    };
  });
  await page.goto('/#activity');
  await expect(page.getByText('Selected text · on this Mac')).toBeVisible();
  await expect(page.getByRole('button', {name: 'Draft a reply ↗'})).toHaveCount(0);
  selection = 'Can you review the marked changes by Friday?';
  await page.reload();
  const memory = page.getByRole('button', {name: 'What Beside knows'});
  const back = page.getByRole('button', {name: 'Ask Beside'});
  await memory.click();
  await page.getByRole('button',{name:'Sources',exact:true}).click();
  await expect(page.getByText(/Last seen just now/i)).toBeVisible();
  await back.click();
  await page.getByRole('button', {name: 'Draft a reply ↗'}).click();
  await expect(page.locator('.reply')).toContainText('Friday');
  await memory.click();
  await page.getByRole('button', {name:'Model',exact:true}).click();
  await page.getByLabel('Answer with').selectOption('ChatGPT');
  await back.click();
  await page.getByRole('textbox').fill('What is an em dash?');
  await page.getByRole('textbox').press('Enter');
  await expect(page.getByText('A public answer.')).toBeVisible();
  await expect(page.locator('.activity-turn')).toHaveCount(2);
  await memory.click();
  await page.getByText('Last question', {exact: true}).click();
  await expect(page.getByText('Answered by ChatGPT')).toBeVisible();
  expect(calls.find(c => c.op === 'typedQuestion')?.args).toEqual({
    text: 'What is an em dash?',
    provider: 'ChatGPT',
    approved: true,
  });

  await page.getByRole('button', {name:'Model',exact:true}).click();
  await expect(page.getByLabel('Answer with')).toHaveValue('');
  await page.getByRole('button', {name:'Sources',exact:true}).click();
  await page
    .getByText('Record a meeting', {exact: true})
    .click();
  await expect(page.getByLabel('Include my microphone')).not.toBeChecked();
  await page.getByRole('button', {name: 'Start recording'}).click();
  await expect(
    page.getByText('Recording your selected meeting window'),
  ).toBeVisible();
  await page
    .getByRole('button', {name: 'Stop recording', exact: true})
    .last()
    .click();
  await page
    .getByRole('button', {name: 'Action items from last meeting'})
    .click();
  await expect(page.locator('.reply').last()).toContainText('by Friday');
  expect(calls.find(c => c.op === 'meetingStart')?.args.microphone).toBe(false);
});

test('a Messages shortcut opens an editable unsent draft without a question or selection', async ({
  page,
}) => {
  const operations: string[] = [];
  let consumed = false;
  await page.exposeFunction(
    'replyIPC',
    (op: string, args: Record<string, unknown>) => {
      operations.push(op);
      if (op === 'messageReplyIntent' && !consumed) {
        consumed = true;
        return {
          id: 'foreground-thread',
          contact: 'Sam',
          incoming: 'when is lunch?',
        };
      }
      if (op === 'draftMessageReply') {
        expect(args).toEqual({id: 'foreground-thread'});
        return {
          answer: 'how about noon?',
          incoming: 'when is lunch?',
          styleCount: 3,
          sources: [],
          epoch: 1,
        };
      }
      if (op === 'activitySettings')
        return {
          scope: 'displays',
          enabled: true,
          sources: [],
          screenPermission: true,
          localModel: 'Inkling',
        };
      if (op === 'activityEpoch') return 1;
      return null;
    },
  );
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener() {},
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: (name: string, req: any) =>
        name === 'notebook'
          ? (window as any).replyIPC(req.operation, req.args)
          : Promise.resolve(1),
    };
  });
  await page.goto('/#activity');
  await expect(page.getByText('Draft for Sam')).toBeHidden();
  const draft = page.getByRole('textbox', {name: 'Unsent reply draft'});
  await expect(draft).toHaveValue('how about noon?');
  await draft.fill('how about 1?');
  await draft.press('Enter');
  await expect(draft).toHaveValue('how about 1?\n');
  expect(
    operations.filter(op =>
      ['activityAsk', 'typedQuestion', 'draftMessageReply'].includes(op),
    ),
  ).toEqual(['draftMessageReply']);
});

test('holding the shortcut keeps the pointed context local and sends only the reviewed search words', async ({
  page,
}) => {
  const calls: Row[] = [];
  let consumed = false;
  await page.exposeFunction('pointIPC', (op: string, args: Row) => {
    if (op === 'voiceIntent' && !consumed) {
      consumed = true;
      return {
        id: 'pinned',
        app: 'Notes',
        line: 'private note line',
        text: 'Look this up',
      };
    }
    if (op === 'activitySettings')
      return {
        enabled: true,
        scope: 'displays',
        sources: [],
        localModel: 'Inkling',
      };
    if (op === 'activityAsk') {
      calls.push(args);
      if (!args.publicQuery)
        return {
          answer: 'Review the words to look up.',
          proposedQuery: 'Apple Notes',
          sources: [],
          pointLine: 'private note line',
          pointApp: 'Notes',
        };
      return {
        answer: 'Use a checklist [1].',
        sources: [],
        webSources: [
          {title: 'Notes guide', url: 'https://support.apple.com/guide/notes'},
        ],
      };
    }
    return null;
  });
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener() {},
    };
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: (name: string, req: any) =>
        name === 'notebook'
          ? (window as any).pointIPC(req.operation, req.args)
          : Promise.resolve(1),
    };
  });
  await page.goto('/#activity');
  const query = page.getByRole('textbox', {name: 'Public search query'});
  await expect(query).toHaveValue('Apple Notes');
  expect(calls).toHaveLength(1);
  await query.fill('Notes checklist shortcut Mac');
  await page.getByRole('button', {name: 'Search the web', exact: true}).click();
  await expect(page.locator('.activity-turn .reply').last()).toContainText('Use a checklist');
  expect(calls[1].publicQuery).toBe('Notes checklist shortcut Mac');
  expect(calls[1].gesture).toBe('pinned');
});

test('checks a train against an earlier ferry email and reveals the exact source', async ({
  page,
}, info) => {
  test.setTimeout(180000);
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    const email =
      'Friday’s last ferry leaves Bayport Pier at 7:15 PM; trains arriving after that miss the crossing.';
    for (const [title, body] of [
      ['Mail · Friday crossing', email],
      [
        'Notes · Books',
        'Bring a paperback for the train ride home.',
      ],
      [
        'Friday train options',
        'Later train to the island ferry: arrives at Bayport Pier at 7:40 PM on Friday.',
      ],
    ])
      await rpc('fixtureActivity', {source: 'display', title, body});
    const journal = await rpc('activityJournal');
    const anchor = journal.events.find((e: Row) => e.title === 'Friday train options');
    expect(anchor).toBeTruthy();
    const asks: Row[] = [];
    await page.exposeFunction(
      'connectionIPC',
      async (op: string, args: Row) => {
        if (op === 'activityAnchor') return anchor;
        if (op === 'activityAsk') {
          const reply = await rpc(op, args);
          asks.push(reply);
          return reply;
        }
        if (
          [
            'activityEpoch',
            'activitySettings',
            'activitySource',
            'activityJournal',
          ].includes(op)
        )
          return rpc(op, args);
        return null;
      },
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, r: any) =>
          cmd === 'notebook'
            ? (window as any).connectionIPC(r.operation, r.args)
            : Promise.resolve(1),
      };
    });
    await page.setViewportSize({width: 420, height: 250});
    await page.goto('/#activity');
    await expect(page.getByLabel(/Remove context/)).toBeVisible();
    const question = page.getByRole('textbox', {name: 'Your question'});
    await question.fill('Does this train connect with the last ferry?');
    await question.press('Enter');
    const answer = page.locator('.activity-turn .reply');
    await expect(answer).toHaveText(
      'No. Your 7:40 PM arrival is 25 minutes after the 7:15 PM departure.',
      {timeout: 120000},
    );
    expect(asks[0].sources.map((s: Row) => s.title)).toEqual([
      'Mail · Friday crossing',
    ]);
    expect(asks[0].supportingLine).toBe(email);
    const evidence = page.locator('.activity-turn .margin-footnote blockquote');
    await expect(evidence).toHaveText(email);
    await expect(evidence).toBeHidden();
    await page.setViewportSize({width: 460, height: 365});
    await page.screenshot({path: info.outputPath('ferry-answer.png')});
    console.log('Grounded connection:', asks[0].answer);
    await page.getByRole('button', {name: /Friday crossing/}).click();
    await expect(evidence).toBeVisible();
    const earlier = await rpc('activityAsk', {
      allowGeneral: true,
      text: 'Does this train connect with the last ferry?',
      selection: 'Friday’s train arrives at Bayport Pier at 6:45 PM.',
    });
    expect(earlier.answer).toBe('Yes, on the timetable. Your 6:45 PM arrival leaves 30 minutes before the 7:15 PM departure.');
    expect(earlier.supportingLine).toBe(email);
    const otherDay = await rpc('activityAsk', {
      allowGeneral: true,
      text: 'Does this train connect with the last ferry?',
      selection: 'Saturday’s train arrives at Bayport Pier at 6:45 PM.',
    });
    expect(otherDay.sources).toEqual([]);
    expect(otherDay.clarifyConnection).toBe(true);
    const dressing = 'Use 4½ tbsp of tahini for the maple dressing.';
    await rpc('fixtureActivity', {source: 'display', title: 'Notes · Maple dressing', body: dressing});
    const recipe = await rpc('activityAsk', {
      allowGeneral: true,
      text: 'Does this relate to anything in my notes? How much tahini did I use?',
      selection: 'I’m making our maple dressing and need the amount of tahini.',
    });
    expect(recipe.answer).toContain('4½ tbsp');
    expect(recipe.supportingLine).toBe(dressing);
    const miss = await rpc('activityAsk', {
      allowGeneral: true,
      text: 'Is this related to anything I saved?',
      selection: 'Parabolic comet orbits and perihelion velocity',
    });
    expect(miss.sources).toEqual([]);
    expect(miss.answer).toContain('couldn’t find');
  } finally {
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});

test('floating reply correction stays with its chat and survives reopening the notebook', async ({
  page,
}) => {
  const fixture = await fixtureNotebook();
  let core = fixture.core;
  let rpc = fixture.rpc;
  try {
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    await rpc('fixtureActivity', {
      source: 'display',
      title: 'Notes · Gallery supplies',
      body: 'Gallery supplies\nFrames: 12 frames\nClips: 8 clips',
    });
    await page.exposeFunction('sampleNotebook', (op: string, args: Row) =>
      op === 'activitySize' ? null : rpc(op, args),
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: Row) =>
          cmd === 'notebook'
            ? (window as any).sampleNotebook(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/#activity');
    const epoch = await rpc('activityEpoch');
    const legacyChat = {
      id: 'bee994ca-aad7-40ad-b177-46ce09961af5',
      title: 'Gallery opening',
      epoch,
      updatedAt: Date.now(),
      turns: [
        {
          question: 'When is the gallery open?',
          answer: 'At noon.',
          sources: [],
          epoch,
        },
      ],
    };
    await page.evaluate(
      (chat) =>
        localStorage.setItem('beside.floating.chats', JSON.stringify([chat])),
      legacyChat,
    );
    await page.reload();
    await page.getByRole('button', {name: 'Chats', exact: true}).click();
    expect(await rpc('floatingChats')).toEqual([]);
    await page.getByRole('button', {name: 'Import earlier chats'}).click();
    await expect(
      page.getByRole('button', {name: /Gallery opening/}),
    ).toBeVisible();
    expect(
      await page.evaluate(() => localStorage.getItem('beside.floating.chats')),
    ).toBeNull();
    await page.getByRole('button', {name: 'New chat', exact: true}).click();
    const q =
      'I have 4 frames and 2 clips at home. From my gallery supplies note, draft a text asking for what I still need to buy.';
    const field = page.getByRole('textbox', {name: 'Your question'});
    await field.fill(q);
    await field.press('Enter');
    await expect(page.locator('.activity-turn')).toContainText(
      '8 frames and 6 clips',
    );
    const actions = page.locator('.activity-turn').last().locator('.turn-actions');
    await page.getByRole('textbox', {name: 'Follow up'}).focus();
    await page.mouse.move(1, 1);
    await expect(actions).toHaveCSS('opacity', '0');
    const correct = actions.getByRole('button', {name: 'Correct this reply'});
    await correct.focus();
    await expect(actions).toHaveCSS('opacity', '1');
    await page.keyboard.press('Enter');
    const rule = 'For frame purchases, I always choose walnut over oak.';
    await page.getByLabel('What should I remember for next time?').fill(rule);
    await page.getByRole('button', {name: 'Save correction'}).click();
    await expect(
      page.getByRole('status').filter({hasText: rule}),
    ).toBeVisible();
    const original = (await rpc('floatingChats'))[0];
    expect(original.turns[0].question).toBe(q);
    expect(
      await page.evaluate(() => localStorage.getItem('beside.floating.chats')),
    ).toBeNull();
    const exited = once(core, 'exit');
    core.stdin.end();
    await exited;
    const restarted = await fixtureNotebook(false, undefined, fixture.folder);
    core = restarted.core;
    rpc = restarted.rpc;
    await page.reload();
    await page.getByRole('button', {name: 'Chats', exact: true}).click();
    await page
      .getByRole('button', {name: /I have 4 frames/})
      .first()
      .click();
    await expect(page.locator('.activity-turn')).toContainText(
      '8 frames and 6 clips',
    );
    expect(
      (await rpc('state')).notes.some((n: Row) => n.text === rule && n.private),
    ).toBeTruthy();
    if (process.env.BESIDE_TEST_INKLING === '1') {
      await page.getByRole('button', {name: 'New', exact: true}).click();
      await field.fill(
        'Choose the frame finish I prefer, oak or walnut. Give my choice in one word.',
      );
      await field.press('Enter');
      await expect(page.locator('.activity-turn .reply')).toContainText(
        /walnut/i,
        {timeout: 90000},
      );
    }
    await rpc('activityForget', {scope: 'all'});
    expect(await rpc('floatingChats')).toEqual([]);
  } finally {
    const done = once(core, 'exit');
    core.stdin.end();
    await done;
    await rm(fixture.folder, {recursive: true, force: true});
  }
});

test('notebook launch recovers from a failed load and opens every workspace section', async ({
  page,
}) => {
  const {rpc, core, folder} = await fixtureNotebook();
  const calls: string[] = [];
  let unavailable = true;
  try {
    await page.exposeFunction(
      'workspaceTest',
      async (op: string, args: Row) => {
        calls.push(op);
        if (op === 'state' && unavailable)
          throw Error('Notebook is temporarily unavailable');
        if (op === 'openActivity') return null;
        return rpc(op, args);
      },
    );
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: Row) =>
          cmd === 'notebook'
            ? (window as any).workspaceTest(p.operation, p.args)
            : Promise.resolve(1),
      };
    });
    await page.goto('/');
    await expect(page.getByRole('alert')).toContainText(
      'temporarily unavailable',
    );
    unavailable = false;
    await page.getByRole('button', {name: 'Try again'}).click();
    await page.getByRole('button', {name: 'Set up later'}).click();
    await page.getByRole('button', {name: 'Skip for now'}).click();
    await expect.poll(async () => (await rpc('state')).setup).toBeTruthy();
    const nav = page.getByRole('navigation', {name: 'Workspace sections'});
    await expect(nav).toBeVisible();
    await expect(nav.getByRole('button', {name: 'Memory'})).toHaveAttribute('aria-current', 'page');
    await nav.getByRole('button', {name: 'Connections'}).click();
    await expect(
      page.getByRole('heading', {name: 'Connected tools'}),
    ).toBeVisible();
    await nav.getByRole('button', {name: 'Chat', exact: true}).click();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.reload();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.goBack();
    await expect(page.getByRole('heading', {name: 'Connected tools'})).toBeVisible();
    await page.goForward();
    await expect(page.getByLabel('Desk sheet')).toBeVisible();
    await page.getByRole('button', {name: /Sheets/}).click();
    await expect(page.getByRole('button', {name: /New sheet/})).toBeVisible();
    await nav.getByRole('button', {name: 'Settings'}).click();
    const local = page.getByRole('tab', {name: 'On-device'});
    await local.focus();
    await local.press('ArrowRight');
    await expect(page.getByRole('tab', {name: 'Cloud'})).toHaveAttribute(
      'aria-selected',
      'true',
    );
    await page.getByRole('button', {name: 'Claude', exact: true}).click();
    await page.getByLabel('API key').fill('example-only');
    await page.getByLabel('Model ID').fill('my-model');
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await expect(page.getByLabel('API key')).toHaveValue('example-only');
    await expect(page.getByLabel('Model ID')).toHaveValue('my-model');
    const toggle = page.getByLabel('Show suggestions');
    await toggle.uncheck();
    await expect
      .poll(async () => (await rpc('proactiveStatus')).settings.enabled)
      .toBe(false);
    await expect(toggle).toBeEnabled();
    await expect(toggle).not.toBeChecked();
    await toggle.click();
    await expect
      .poll(async () => (await rpc('proactiveStatus')).settings.enabled)
      .toBe(true);
    await expect(toggle).toBeChecked();
    await nav.getByRole('button', {name: 'Memory'}).click();
    await page.getByRole('button', {name: 'Write an entry'}).click();
    await page.getByRole('button', {name: 'Cancel', exact: true}).click();
    await page.getByRole('button', {name: 'Open companion'}).click();
    expect(calls).toContain('openActivity');
    await page.goto('/#settings');
    await expect(nav.getByRole('button', {name: 'Settings'})).toHaveAttribute('aria-current', 'page');
    await page.goto('/');
    await expect(nav.getByRole('button', {name: 'Memory'})).toHaveAttribute('aria-current', 'page');
  } finally {
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});

test('a suggestion waits for the user and keeps the screen that prompted it', async ({
  page,
}, info) => {
  let finish!: () => void;
  const allowFinalRender = new Promise<void>(resolve => { finish = resolve; });
  let askedAt = 0;
  let verifiedMs: number | undefined;
  let firstWordMs: number | undefined;
  const {core, rpc, folder} = await fixtureNotebook(false, undefined, undefined, event => {
    const {payload} = event;
    if (payload.phase === 'source-verified') verifiedMs = Date.now() - askedAt;
    if (payload.delta?.trim() && firstWordMs === undefined) firstWordMs = Date.now() - askedAt;
    void page.evaluate(data => (window as any).activityDelta?.(data), payload).catch(() => {});
  });
  try {
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    await rpc('fixtureActivity', {
      source: 'display',
      age: 600,
      title: 'Notes · Gallery installation',
      body: 'Bring the indigo panels through the west entrance before the loading dock closes at 4 pm.',
    });
    const observedAt = Date.now();
    await rpc('fixtureActivity', {
      source: 'display',
      title: 'Screen 1 · Mail active',
      body: 'When should we deliver the indigo panels to the loading dock?',
    });
    let suggestion: Row = {};
    await expect.poll(async () => {
      suggestion = (await rpc('proactiveStatus')).nudge || {};
      return suggestion.kind;
    }, {timeout: 8000}).toBe('memory');
    console.log('Captured text to suggestion ready (ms)', Date.now() - observedAt);
    expect(suggestion.kind).toBe('memory');
    const questions: Row[] = [];
    await page.exposeFunction('suggestionTest', async (op: string, args: Row) => {
      if (op === 'activityAnchor')
        return {id: 'different-window', label: 'Calendar'};
      if (op === 'activityAsk') {
        questions.push(args);
        askedAt = Date.now();
        const result = await rpc(op, args);
        console.log('Local suggestion timing (ms)', {verifiedMs, firstWordMs, completeMs: Date.now() - askedAt});
        await allowFinalRender;
        return result;
      }
      if (['activitySize', 'activityIntent', 'takeSelection'].includes(op))
        return null;
      return rpc(op, args);
    });
    await page.addInitScript(() => {
      const callbacks: Function[] = [];
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener() {},
      };
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: (callback: Function) => {
          callbacks.push(callback);
          return callbacks.length - 1;
        },
        unregisterCallback() {},
        invoke: (cmd: string, p: Row) => {
          if (cmd === 'notebook') return (window as any).suggestionTest(p.operation, p.args);
          if (cmd === 'plugin:event|listen' && p.event === 'activity-delta')
            (window as any).activityDelta = (payload: Row) => callbacks[p.handler]({payload});
          return Promise.resolve(1);
        },
      };
    });
    await page.setViewportSize({width: 480, height: 360});
    await page.goto('/#activity');
    const offer = page.getByRole('complementary', {
      name: 'Proactive suggestion',
    });
    await expect(offer).toContainText('west entrance');
    expect(questions).toHaveLength(0);
    const field = page.getByRole('textbox', {name: 'Your question'});
    await field.fill('A question of my own');
    await expect(offer).toBeHidden();
    await field.fill('');
    await expect(page.getByRole('alert')).toHaveCount(0);
    await page.screenshot({path: info.outputPath('suggestion.png')});
    await offer.getByRole('button', {name: /Show connection/}).click();
    await expect(page.getByLabel('Saved screen passage')).toContainText(suggestion.detail);
    await expect(page.locator('.assistant-stream')).toContainText(/\S/, {timeout: 120000});
    finish();
    await expect(page.locator('.activity-turn .reply')).toContainText(
      /west entrance|4 pm/i,
      {timeout: 120000},
    );
    expect(questions[0].anchor).toBe(suggestion.eventId);
    expect(questions[0].relatedSource).toBe(suggestion.sourceId);
    expect(questions[0].relatedLine).toBe(suggestion.detail);
    expect(questions[0].history).toEqual([]);
    await expect.poll(async () => (await rpc('floatingChats')).find((chat: Row) =>
      chat.turns.some((turn: Row) => turn.id === questions[0].turn),
    )?.id).toBe(questions[0].chatId);
    expect((await rpc('proactiveStatus')).nudge).toBeNull();
    await expect(offer).toBeHidden();
  } finally {
    finish();
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});

for (const scheme of ['light', 'dark'] as const) {
  test(`floating memory keeps the ink mark clear in ${scheme} mode`, async ({page}, info) => {
    await page.emulateMedia({colorScheme: scheme});
    await page.setViewportSize({width: 390, height: 440});
    await page.addInitScript(() => {
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: async (_: string, p: Row) => {
          if (p?.operation === 'activitySettings') return {enabled: true, localModel:'Inkling'};
          if (p?.operation === 'activityJournal') return {
            events: [{
              id: 'timetable', label: 'Notes', title: 'Friday train tickets',
              snippet: 'Maple Express arrives at 6:10 PM.',
              stamp: 1795616100, hasPreview: true,
            }],
            habits: [], remembered: [{id:'deadline', text:'Reach the pier by 6:30 PM on Friday.'}],
          };
          return null;
        },
      };
    });
    await page.goto('/#activity');
    const mark = page.getByRole('button', {name: 'What Beside knows'});
    await mark.click();
    await expect(page.getByText('Friday train tickets', {exact:true})).toBeVisible();
    await mark.hover();
    await expect(mark.locator('.signature')).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    await expect(mark).not.toHaveAttribute('title', /.+/);
    const ask = page.getByRole('button', {name:'Ask Beside'});
    await expect(ask).toHaveText('Ask');
    await expect(ask).toHaveCSS('white-space', 'nowrap');
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({path: info.outputPath('memory-' + scheme + '.png')});
    await page.keyboard.press('Tab');
    await page.keyboard.press('Shift+Tab');
    await expect(mark).toBeFocused();
    await expect(mark).toHaveCSS('outline-style', 'solid');
    await ask.click();
    await expect(page.locator('.screen-composer textarea')).toBeVisible();
  });
}

test('floating meal plan recalculates guests and pantry, then copies the revised list', async ({page, context}) => {
  const {core, rpc, folder} = await fixtureNotebook();
  try {
    if (process.env.BESIDE_TEST_INKLING === '1') await rpc('fixtureInkling');
    else await rpc('model', {runtime: 'Ollama', name: 'llama3.2:3b'});
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    await rpc('fixtureActivity', {
      source: 'display', title: 'Notes · Weeknight dinners',
      body: 'Weeknight dinners\nMonday soup for two\n• 1 can of beans\nThursday tacos for two\n• 2 cans of beans\n• 4 tortillas\nPantry: 1 can of beans.',
    });
    await page.exposeFunction('mealNotebook', (op: string, args: Row) =>
      op === 'activitySize' ? null : rpc(op, args));
    await page.addInitScript(() => {
      (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
      (window as any).__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: Row) => cmd === 'notebook'
          ? (window as any).mealNotebook(p.operation, p.args) : Promise.resolve(1),
      };
    });
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    await page.goto('/#activity');
    const composer = page.getByRole('textbox', {name: /^(Your question|Follow up)$/});
    const reply = page.locator('.activity-turn .reply').last();
    for (const [question, expected] of [
      ['Make a shopping list from my meal plan.', 'beans — 2 cans'],
      ['Make Thursday dinner for four.', 'beans — 4 cans'],
      ['Actually, I have three cans of beans.', 'beans — 2 cans'],
    ]) {
      await composer.fill(question);
      await composer.press('Enter');
      await expect(reply).toContainText(expected);
    }
    await expect(reply.getByRole('listitem')).toHaveCount(2);
    await expect(reply).toContainText('tortillas — 8');
    await page.getByRole('button', {name: 'Copy list'}).last().click();
    await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('beans — 2 cans');
    const chats = await rpc('floatingChats');
    expect(chats[0].turns.at(-1).mealTask.stock['can:bean']).toEqual(3);
  } finally {
    const closed = once(core, 'close');
    core.stdin.end();
    await closed;
    await rm(folder, {recursive: true, force: true});
  }
});
