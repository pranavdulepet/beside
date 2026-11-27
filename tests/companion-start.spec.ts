import {expect, test, type Page} from '@playwright/test';
import {once} from 'node:events';
import {rm} from 'node:fs/promises';
import {fixtureNotebook} from './fixture';
import type {Row} from '../src/api';

async function openCompanion(page: Page, rpc: (op: string, args: Row) => Promise<unknown>) {
  await page.exposeFunction('companionRpc', rpc);
  await page.addInitScript(() => {
    (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
    (window as any).__TAURI_INTERNALS__ = {
      transformCallback: () => 1, unregisterCallback() {},
      invoke: (command: string, payload: Row) => command === 'notebook'
        ? (window as any).companionRpc(payload.operation, payload.args)
        : Promise.resolve(1),
    };
  });
  await page.setViewportSize({width: 420, height: 430});
  await page.goto('/#activity');
}

async function closeNotebook(book: Awaited<ReturnType<typeof fixtureNotebook>>) {
  const closed = once(book.core, 'close');
  book.core.stdin.end();
  await closed;
  await rm(book.folder, {recursive: true, force: true});
}

const nativeWindow = new Set(['activitySize', 'activityIntent', 'activityAnchor', 'takeSelection']);

test('an empty notebook can start quietly and keeps the decision after a relaunch', async ({page}, info) => {
  const book = await fixtureNotebook();
  const calls: string[] = [];
  try {
    await openCompanion(page, async (op, args) => {
      calls.push(op);
      return nativeWindow.has(op) ? null : book.rpc(op, args);
    });
    const setup = page.getByRole('region', {name: 'Set up Beside'});
    await expect(setup).toBeVisible();
    await expect(page.getByRole('button', {name: 'Choose a model file…'})).toBeVisible();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toHaveCount(0);
    await expect(setup.getByText('Checking this Mac')).toHaveCount(0);
    await setup.screenshot({path: info.outputPath('first-launch.png')});
    await setup.getByRole('button', {name: 'Set up later'}).click();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toBeFocused();
    expect((await book.rpc('state')).setup).toBe(true);
    expect((await book.rpc('activitySettings')).enabled).toBe(false);
    expect(calls).not.toContain('activityPermission');
    expect(calls).not.toContain('downloadBundled');
    await page.reload();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toBeVisible();
    await expect(setup).toHaveCount(0);
  } finally { await closeNotebook(book); }
});

test('a public topic can be clarified without a model, and a private follow-up stays local', async ({page}) => {
  const book = await fixtureNotebook();
  try {
    await book.rpc('companionSetup', {step: 'ready'});
    let last: Row = {};
    await openCompanion(page, async (op, args) => {
      if (nativeWindow.has(op)) return null;
      if (op === 'activityAsk') last = args;
      return book.rpc(op, args);
    });
    const field = page.getByRole('textbox', {name: /Your question|Follow up/});
    await field.fill('Look this up');
    await field.press('Enter');
    await expect(page.locator('.activity-turn .reply').last()).toContainText('What public topic');
    await field.fill('Remember that I prefer short replies.');
    await field.press('Enter');
    await expect.poll(() => last.awaitingSearchTopic).toBe(true);
    await expect(page.locator('.activity-turn .reply').last()).toContainText(/remember|saved|short replies/i);
    expect(last.publicQuery).toBeUndefined();
  } finally {
    await closeNotebook(book);
  }
});

test('screen memory waits for macOS and an explicit start, then works across reopen', async ({page}) => {
  const book = await fixtureNotebook();
  try {
    await book.rpc('companionSetup', {step: 'screen'});
    let allowed = false;
    const changes: Row[] = [];
    await openCompanion(page, async (op, args) => {
      if (nativeWindow.has(op)) return null;
      if (op === 'activityPermission') return false;
      if (op === 'activityConfigure') changes.push(args);
      const value = await book.rpc(op, args);
      return op === 'activitySettings' ? {...value, screenPermission: allowed} : value;
    });
    await page.getByRole('button', {name: 'Allow Screen Recording'}).click();
    await expect(page.getByRole('status')).toContainText('System Settings');
    expect(changes).toHaveLength(0);
    await page.reload();
    await expect(page.getByRole('button', {name: 'Allow Screen Recording'})).toBeVisible();
    allowed = true;
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    const start = page.getByRole('button', {name: 'Turn on screen memory'});
    await expect(start).toBeVisible();
    expect((await book.rpc('activitySettings')).enabled).toBe(false);
    await start.click();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toBeVisible();
    expect(changes).toEqual([{scope: 'displays'}, {enabled: true}]);
    expect((await book.rpc('activitySettings')).companionSetup).toBe('ready');
    await page.reload();
    await expect(page.getByRole('region', {name: 'Set up Beside'})).toHaveCount(0);
  } finally { await closeNotebook(book); }
});

test('first launch can use an already downloaded Inkling with a live readiness check', async ({page}) => {
  test.skip(process.env.BESIDE_TEST_INKLING !== '1', 'Needs a previously downloaded local Inkling.');
  const book = await fixtureNotebook();
  const calls: string[] = [];
  try {
    const choices = (await book.rpc('modelDownloadStatus')).choices;
    test.skip(!choices.some((m: Row) => m.label === 'Inkling 2 Mini' && m.installedCopy), 'Needs Inkling in a standard notebook.');
    await openCompanion(page, async (op, args) => {
      calls.push(op);
      return nativeWindow.has(op) ? null : book.rpc(op, args);
    });
    await page.getByRole('button', {name: 'Use Inkling 2 Mini'}).click();
    await expect(page.getByRole('heading', {name: 'Remember what you’ve seen?'})).toBeVisible({timeout: 120000});
    await page.getByRole('button', {name: 'Keep it off'}).click();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toBeFocused();
    const state = await book.rpc('activitySettings');
    expect(state.localModel.name).toMatch(/Inkling/);
    expect(state.enabled).toBe(false);
    expect(calls).toContain('useInstalledModel');
    expect(calls).not.toContain('downloadBundled');
  } finally { await closeNotebook(book); }
});

test('reading only on summon survives a restart and never starts the observer', async ({page}) => {
  let book = await fixtureNotebook();
  let screenPermission = false;
  try {
    await book.rpc('companionSetup', {step: 'screen'});
    await openCompanion(page, async (op, args) => {
      if (nativeWindow.has(op)) return null;
      if (op === 'activityPermission') return screenPermission;
      const result = await book.rpc(op, args);
      return op === 'activitySettings' ? {...result, screenPermission} : result;
    });
    await page.getByRole('button', {name: 'Only when I ask', exact: true}).click();
    await expect(page.getByRole('heading', {name: 'Read only when you ask.'})).toBeVisible();
    await page.getByRole('button', {name: 'Allow Screen Recording'}).click();
    await expect(page.getByRole('status')).toContainText('System Settings');
    expect((await book.rpc('activitySettings')).scope).not.toBe('on-demand');
    screenPermission = true;
    await page.reload();
    await page.getByRole('button', {name: 'Use the shortcut only'}).click();
    await expect(page.getByRole('textbox', {name: 'Your question'})).toBeFocused();
    expect(await book.rpc('activitySettings')).toMatchObject({scope: 'on-demand', enabled: false, companionSetup: 'ready'});
    const closed = once(book.core, 'close');
    book.core.stdin.end();
    await closed;
    book = await fixtureNotebook(false, undefined, book.folder);
    await page.reload();
    await book.rpc('fixtureActivity', {source: 'display', title: 'Unasked window', body: 'Weekend brush lettering starts at 10 am.'});
    expect((await book.rpc('activityJournal')).events).toEqual([]);
    await page.getByRole('button', {name: 'What Beside knows'}).click();
    await page.getByRole('navigation', {name: 'Beside menu'}).getByRole('button', {name: 'Sources'}).click();
    const sources = page.getByRole('region', {name: 'Screen access'});
    await expect(sources.getByText('Only when you ask', {exact: true})).toBeVisible();
    await sources.getByRole('button', {name: 'Turn off', exact: true}).click();
    await expect(sources.getByText('Observation paused')).toBeVisible();
    expect(await book.rpc('activitySettings')).toMatchObject({scope: 'selected', enabled: false});
    await expect(book.rpc('activityAsk', {text: 'What does that say?', onDemand: true, gesture: 'revoked'})).rejects.toThrow(/no longer available/);
  } finally { await closeNotebook(book); }
});
