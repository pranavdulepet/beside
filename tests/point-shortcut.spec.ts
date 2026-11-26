import {expect, test} from '@playwright/test';
import type {Row} from '../src/api';
import {fixtureNotebook} from './fixture';

test('inspect the packing list and weather at checkout without leaving the answer', async ({
  page,
}, info) => {
  const {core, rpc} = await fixtureNotebook();
  const title = 'Athens packing list';
  const body = 'Athens packing list\n2 December\nRain jacket\nSneakers\nBook in Greek: Οδύσσεια';
  const url = 'https://api.open-meteo.com/v1/forecast?latitude=37.94&longitude=23.94';
  const weather = {
    title: 'Open-Meteo · ATH · Dec 2',
    quote: '4–8 °C near ATH on Dec 2.',
    url,
    basis: 'Beside used the arrival airport and outbound date from your booking.',
  };
  const visits: string[] = [];
  const sourceReads: string[] = [];
  try {
    await rpc('activityConfigure', {scope: 'displays'});
    await rpc('activityConfigure', {enabled: true});
    await rpc('fixtureActivity', {source: 'display', title, body});
    const epoch = await rpc('activityEpoch');
    const id = (await rpc('activityJournal')).events.find((e: Row) => e.title === title).id;
    await page.exposeFunction('tripIPC', async (op: string, args: Row) => {
      if (op === 'activitySettings')
        return {companionSetup: 'ready', enabled: true, localModel: 'Inkling 2 Mini'};
      if (op === 'activityEpoch') return epoch;
      if (op === 'activityAsk')
        return {
          answer: 'Keep the included 20 kg bag.\n\nYour suitcase and the items in your December packing list come to about 15 kg. The fare includes one checked bag. Your list already includes a rain jacket for the forecast in Athens.',
          workKind: 'baggage',
          sources: [
            {id: 'bag-offer', title: 'Check-in bags · Ryanair', label: 'Booking'},
            {id, title, role: 'packing'},
          ],
          webSources: [weather],
          epoch,
        };
      if (op === 'activitySource') {
        sourceReads.push(args.id);
        return rpc(op, args);
      }
      if (op === 'visitPublicSource') {
        visits.push(args.url);
        return true;
      }
      if (op === 'floatingChats') return [];
      return null;
    });
    await page.addInitScript(() => {
      const native = window as any;
      native.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
      native.__TAURI_INTERNALS__ = {
        transformCallback: () => 1,
        invoke: (cmd: string, p: Row) =>
          cmd === 'notebook' ? native.tripIPC(p.operation, p.args) : Promise.resolve(1),
      };
    });
    await page.setViewportSize({width: 460, height: 400});
    await page.goto('/#activity');
    const input = page.getByRole('textbox', {name: 'Your question'});
    await input.fill('Should I add another checked bag?');
    await input.press('Enter');
    const note = page.getByRole('button', {name: title, exact: true});
    const forecast = page.getByRole('button', {name: weather.title, exact: true});
    await expect(note).toHaveAttribute('aria-expanded', 'false');
    expect(sourceReads).toEqual([]);
    await note.press('Enter');
    const savedNote = page.getByRole('region', {name: title});
    await expect(savedNote).toContainText('Οδύσσεια');
    await expect(savedNote).toBeInViewport({ratio: 0.98});
    expect(sourceReads).toEqual([id]);
    await forecast.click();
    const detail = page.getByRole('region', {name: weather.title});
    await expect(detail).toContainText(weather.quote);
    await expect(detail).toContainText(weather.basis);
    await expect(detail).toBeInViewport({ratio: 0.98});
    await expect(note).toHaveAttribute('aria-expanded', 'false');
    await expect(page.getByText('Keep the included 20 kg bag.', {exact: true})).toBeVisible();
    await expect(page.getByRole('textbox', {name: 'Follow up'})).toBeVisible();
    expect(visits).toEqual([]);
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({colorScheme});
      await page.screenshot({
        path: info.outputPath('forecast-' + colorScheme + '.png'),
        animations: 'disabled',
      });
    }
    await detail.getByRole('button', {name: 'Open forecast'}).click();
    expect(visits).toEqual([url]);
    await forecast.click();
    await expect(detail).toBeHidden();
    await rpc('activityForget', {scope: 'event', id});
    await note.click();
    await expect(page.getByRole('alert')).toContainText('forgotten');
    await expect(page.getByText('Οδύσσεια', {exact: false})).toHaveCount(0);
  } finally {
    core.stdin.end();
  }
});

test('a bag offer can open its missing permission from the answer', async ({page}) => {
  let granted = false;
  const requests: Row[] = [];
  await page.exposeFunction('bookingIPC', (op: string, args: Row) => {
    if (op === 'activitySettings') return {companionSetup: 'ready', enabled: true, sitePermission: granted};
    if (op === 'activityAsk') return {answer: 'Allow Accessibility to match each bag to its price.', workKind: 'baggage-access', sources: []};
    if (op === 'activityPermission') {
      requests.push(args);
      granted = true;
      return granted;
    }
    if (op === 'floatingChats') return [];
    return null;
  });
  await page.addInitScript(() => {
    const native = window as any;
    native.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
    native.__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: (command: string, payload: Row) => command === 'notebook'
        ? native.bookingIPC(payload.operation, payload.args) : Promise.resolve(1),
    };
  });
  await page.goto('/#activity');
  const compose = page.getByRole('textbox', {name: 'Your question'});
  await compose.fill('Should I add another checked bag?');
  await compose.press('Enter');
  const permission = page.getByRole('button', {name: 'Allow Accessibility…'});
  await expect(permission).toBeVisible();
  expect(requests).toEqual([]);
  await permission.click();
  await expect.poll(() => requests).toEqual([{kind: 'sites'}]);
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await expect(permission).toBeHidden();
});

test('cold and reused panels reveal the current task, preserve early typing, and keep saved chats', async ({page}) => {
  const asks: Row[] = [];
  const history = new Map<string, Row>();
  let selection: Promise<string> | undefined;
  let selectionRead = () => {};
  let finishFirstSave = () => {};
  const firstSave = new Promise<void>(resolve => {finishFirstSave = resolve});
  let receipt: Row | null = {id: 'old-window', app: 'TextEdit', typed: true, hasImage: true};
  let delivery = Promise.resolve();
  let announceRead = () => {};
  function nextWindow(id: string, app: string) {
    receipt = {id, app, typed: true, hasImage: true};
    let release = () => {};
    delivery = new Promise<void>(resolve => {release = resolve});
    const reading = new Promise<void>(resolve => {announceRead = resolve});
    return {release, reading};
  }
  await page.exposeFunction('shortcutIPC', async (op: string, args: Row) => {
    if (op === 'pendingActivityPresentation') return {id: 1, cold: true, anchor: null};
    if (op === 'takeSelection') {
      const current = selection;
      selection = undefined;
      if (current) selectionRead();
      return current ? await current : null;
    }
    if (op === 'voiceIntent') {
      const point = receipt;
      receipt = null;
      if (point) {announceRead(); await delivery}
      return point;
    }
    if (op === 'activitySettings') return {companionSetup: 'ready', enabled: true, scope: 'displays', sources: []};
    if (op === 'activityEpoch') return 'epoch';
    if (op === 'floatingChats') return [...history.values()];
    if (op === 'saveFloatingChat') {
      if (!history.size) await firstSave;
      for (const chat of args.chats) history.set(chat.id, chat);
      return null;
    }
    if (op === 'activityAsk') {
      asks.push(args);
      return {answer: 'Read ' + (args.gesture || args.selection), sources: [], epoch: 'epoch'};
    }
    return null;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: Row) => void>();
    const listeners = new Map<string, number[]>();
    let nextId = 0;
    const native = window as any;
    native.firstFrames = [];
    native.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
    native.__TAURI_INTERNALS__ = {
      transformCallback(fn: (event: Row) => void) {callbacks.set(++nextId, fn); return nextId},
      invoke(command: string, payload: Row) {
        if (command === 'notebook' && payload.operation === 'activityPresentationReady') {
          native.firstFrames.push({
            id: payload.args.id,
            visibleText: document.querySelector('main')?.innerText,
            composer: document.activeElement?.closest('label')?.querySelector('span')?.textContent,
          });
        }
        if (command === 'notebook') return native.shortcutIPC(payload.operation, payload.args);
        if (command === 'plugin:event|listen') {
          listeners.set(payload.event, [...(listeners.get(payload.event) || []), payload.handler]);
          return Promise.resolve(payload.handler);
        }
        return Promise.resolve();
      },
    };
    native.deliverShortcutEvent = (name: string, payload?: Row) => {
      for (const id of listeners.get(name) || []) callbacks.get(id)?.({event: name, id, payload});
    };
  });
  await page.goto('/#activity');
  const chip = page.getByRole('button', {name: 'Remove captured screen context'});
  const input = page.locator('.activity-question textarea');
  await expect(chip).toContainText('TextEdit');
  await expect.poll(() => page.evaluate(() => (window as any).firstFrames.map((frame: Row) => frame.id))).toEqual([1]);
  await input.fill('Read the old window');
  await input.press('Enter');
  await expect(page.getByText('Read old-window', {exact:true})).toBeVisible();
  const oldChat = asks[0].chatId;

  const note = nextWindow('note-window', 'Notes');
  await input.press('Escape');
  await page.evaluate(() => {
    const native = window as any;
    native.deliverShortcutEvent('activity-expand', {id: 2, anchor: null});
    native.deliverShortcutEvent('voice-ready');
  });
  await note.reading;
  const first = await page.evaluate(() => (window as any).firstFrames.at(-1));
  expect(first).toMatchObject({id: 2, composer: 'Your question'});
  expect(first.visibleText).not.toContain('Read old-window');
  await expect(page.getByText('Read old-window', {exact: true})).toHaveCount(0);
  await input.fill('What time is the workshop?');
  finishFirstSave();
  note.release();
  await expect(chip).toContainText('Notes');
  await expect(input).toHaveValue('What time is the workshop?');
  await input.press('Enter');
  await expect(page.getByText('Read note-window', {exact:true})).toBeVisible();
  expect(asks[1]).toMatchObject({text:'What time is the workshop?', gesture:'note-window', history:[]});
  expect(asks[1].chatId).not.toBe(oldChat);

  // A reused WKWebView can regain focus while its native event is still queued.
  const mail = nextWindow('mail-window', 'Mail');
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await input.fill('Which file did the printer ask for?');
  await input.press('Enter');
  await mail.reading;
  expect(asks).toHaveLength(2);
  mail.release();
  await expect(page.getByText('Read mail-window', {exact:true})).toBeVisible();
  expect(asks[2]).toMatchObject({text:'Which file did the printer ask for?', gesture:'mail-window', history:[]});
  expect(asks[2].chatId).not.toBe(asks[1].chatId);

  await page.getByRole('button', {name: 'Chats', exact: true}).click();
  await page.getByRole('button', {name: /Read the old window/}).click();
  await expect(page.getByText('Read old-window', {exact: true})).toBeVisible();
  expect(history.get(oldChat)?.turns[0].answer).toBe('Read old-window');

  let deliverSelection = (_text: string) => {};
  selection = new Promise<string>(resolve => {deliverSelection = resolve});
  const readingSelection = new Promise<void>(resolve => {selectionRead = resolve});
  await page.getByRole('button', {name: 'Hide assistant'}).click();
  await page.evaluate(() => {
    const native = window as any;
    native.deliverShortcutEvent('activity-expand', {id: 3, anchor: null});
    native.deliverShortcutEvent('selection-ready');
  });
  await readingSelection;
  await input.fill('Check this sentence.');
  await input.press('Enter');
  expect(asks).toHaveLength(3);
  deliverSelection('The wall was timber.');
  await expect(page.getByText('Read The wall was timber.', {exact: true})).toBeVisible();
  expect(asks[3]).toMatchObject({text:'Check this sentence.', selection:'The wall was timber.', history:[]});
  expect(asks[3].chatId).not.toBe(oldChat);
});
