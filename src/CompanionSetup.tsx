import {useEffect, useState} from 'react';
import {call, type Row} from './api';
import {BesideMark} from './Ink';

type Choice = {op: string; args: Row; label: string; title: string};

function recommended(download: Row, fit: Row, installed: Row[], skipCopy: boolean): Choice | null {
  const choices: Row[] = download.choices || [download];
  const ready = choices.find(m => m.fits !== false && (m.cached || (!skipCopy && m.installedCopy)));
  if (ready) return {
    op: ready.cached ? 'downloadBundled' : 'useInstalledModel',
    args: {label: ready.label}, label: ready.label, title: 'Use ' + ready.label,
  };
  const fits = installed.filter(m => !m.bytes || m.bytes * 1.3 + 3e9 < fit.memory);
  const running = fits.find(m => m.name === fit.recommendation?.name) || fits.at(-1);
  if (running) return {op: 'model', args: {name: running.name, runtime: running.runtime}, label: running.name, title: 'Use ' + running.name};
  const next = choices.find(m => m.fits !== false && fit.freeDisk >= m.required);
  if (!next) return null;
  const remaining = Math.max(0, next.total - (next.bytes || 0));
  return {
    op: 'downloadBundled', args: {label: next.label}, label: next.label,
    title: (next.bytes ? 'Resume ' : 'Download ') + next.label + ' · ' + (remaining / 1e9).toFixed(1) + ' GB',
  };
}

export function CompanionSetup({settings, finish}: {
  settings: Row;
  finish: (next: Row, destination?: 'sources') => void;
}) {
  const modelStep = settings.companionSetup === 'model';
  const pointStep = settings.companionSetup === 'point';
  const [choice, setChoice] = useState<Choice | null>(null);
  const [skipCopy, setSkipCopy] = useState(false);
  const [checking, setChecking] = useState(true);
  const [work, setWork] = useState('');
  const [progress, setProgress] = useState<Row | null>(null);
  const [error, setError] = useState('');
  const [requested, request] = useState(false);
  useEffect(() => {
    if (!modelStep) return;
    let live = true;
    setChecking(true);
    void Promise.all([call('modelDownloadStatus'), call('fit'), call('models')])
      .then(([download, fit, models]) => {
        if (live) setChoice(recommended(download, fit, models, skipCopy));
      })
      .catch(e => { if (live) setError(String(e)); })
      .finally(() => { if (live) setChecking(false); });
    return () => { live = false; };
  }, [modelStep, skipCopy]);
  useEffect(() => {
    if (work !== 'download') return;
    let live = true;
    let timer: number;
    async function check() {
      try {
        const status = await call('modelDownloadStatus');
        if (live) setProgress(status.choices?.find((m: Row) => m.label === choice?.label) || status);
      } catch { /* The download itself reports failures. */ }
      if (live) timer = window.setTimeout(check, 900);
    }
    void check();
    return () => { live = false; window.clearTimeout(timer); };
  }, [work, choice?.label]);
  async function step(next: 'screen' | 'point' | 'ready', destination?: 'sources') {
    await call('companionSetup', {step: next});
    finish(await call('activitySettings'), destination);
  }
  async function run(label: string, operation: () => Promise<void>) {
    if (work) return;
    setWork(label);
    setError('');
    try { await operation(); }
    catch (e) { setError(String(e)); }
    finally { setWork(''); }
  }
  async function useModel(source: Choice | null) {
    const download = source?.op === 'downloadBundled' && !source.title.startsWith('Use ');
    await run(download ? 'download' : 'model', async () => {
      let model: Row | null;
      try { model = await call(source?.op || 'importModel', source?.args); }
      catch (e) {
        if (source?.op === 'useInstalledModel') setSkipCopy(true);
        throw e;
      }
      if (model?.name) await step(settings.enabled ? 'ready' : 'screen');
    });
  }
  async function allowScreen(scope: 'displays' | 'on-demand') {
    await run('screen', async () => {
      await call('companionSetup', {step: scope === 'on-demand' ? 'point' : 'screen'});
      if (!settings.screenPermission && !(await call('activityPermission', {kind: 'apps'}))) {
        request(true);
        finish(await call('activitySettings'));
        return;
      }
      await call('activityConfigure', {scope});
      if (scope === 'displays') await call('activityConfigure', {enabled: true});
      await step('ready');
    });
  }
  return (
    <section className="companion-start" aria-label="Set up Beside">
      <h1>{modelStep ? 'Ask from the app you’re in.' : pointStep ? 'Read only when you ask.' : 'Remember what you’ve seen?'}</h1>
      <p>{modelStep
        ? 'Use a model on this Mac to answer questions beside your work.'
        : pointStep
          ? 'The shortcut lets a local model read the window under your pointer. Your chat saves the question, reply and quoted line. The screenshot stays in memory until you dismiss Beside.'
          : 'Screen memory saves passages from all your displays on this Mac, including visible private windows. For a single window at a time, choose “Only when I ask.”'}</p>
      {!modelStep && !pointStep && settings.brain?.mode === 'cloud' && <small>When you ask with screen memory, permitted context can go to {settings.brain.providerLabel}.</small>}
      {modelStep ? <>
        {choice && <button className="primary" type="button" disabled={!!work || checking} onClick={() => void useModel(choice)}>
          {choice.title}
        </button>}
        <div className="setup-links">
          <button type="button" className={choice || checking ? 'bar-link' : 'primary'} disabled={!!work} onClick={() => void useModel(null)}>Choose a model file…</button>
          <button type="button" className="bar-link" disabled={!!work} onClick={() => void run('finish', () => step('ready'))}>Set up later</button>
        </div>
        {!checking && !choice && !error && <small>You can use a local GGUF file or connect a provider in Model.</small>}
      </> : <>
        <button className="primary" type="button" disabled={!!work} onClick={() => void allowScreen(pointStep ? 'on-demand' : 'displays')}>
          {settings.screenPermission ? pointStep ? 'Use the shortcut only' : 'Turn on screen memory' : 'Allow Screen Recording'}
        </button>
        <div className="setup-links">
          {!pointStep && <button className="bar-link" type="button" disabled={!!work} onClick={() => void run('point', () => step('point'))}>Only when I ask</button>}
          <button className="bar-link" type="button" disabled={!!work} onClick={() => void run('finish', async () => {
            await call('activityConfigure', {scope: 'selected'});
            await step('ready', 'sources');
          })}>Choose specific apps</button>
          <button className="bar-link" type="button" disabled={!!work} onClick={() => void run('finish', async () => {
            if (settings.scope === 'on-demand') await call('activityConfigure', {scope: 'selected'});
            if (settings.enabled) await call('activityConfigure', {enabled: false});
            await step('ready');
          })}>Keep it off</button>
        </div>
        {requested && !settings.screenPermission && <p role="status">In System Settings, turn on Screen Recording for Beside, then return here.</p>}
      </>}
      {(work === 'model' || work === 'download' || (modelStep && checking && !error)) && <div className="setup-progress" role="status">
        {work === 'download' ? 'Installing ' + choice?.label : work === 'model' ? 'Checking the model on this Mac' : 'Checking this Mac'}
        <BesideMark points={settings.mark} animated size={17} />
        {work === 'download' && !!progress?.bytes && <progress aria-label="Model download" value={progress.bytes} max={progress.total} />}
      </div>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
