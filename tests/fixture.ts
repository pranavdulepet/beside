import {spawn} from 'node:child_process';
import {mkdir, mkdtemp, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {createInterface} from 'node:readline';

export async function fixtureNotebook(
  agents = false,
  datedHistory?: {markdown: string; scores: object[]},
  existingFolder?: string,
  event?: (message: any) => void,
) {
  const folder = existingFolder ?? (await mkdtemp(join(tmpdir(), 'beside-ui-')));
  if (datedHistory) {
    await mkdir(join(folder, 'scroll'), {recursive: true});
    await writeFile(join(folder, 'scroll/deciding.md'), datedHistory.markdown);
    await writeFile(
      join(folder, 'predictions.json'),
      JSON.stringify(datedHistory.scores),
    );
  }
  const core = spawn(
    'src-tauri/target/debug/beside',
    [
      '--fixture-rpc',
      '--notebook',
      folder,
      ...(agents ? ['--fixture-agents'] : []),
      ...(datedHistory ? ['--fixture-predictions'] : []),
    ],
    {stdio: ['pipe', 'pipe', 'inherit']},
  );
  const pending: ((s: string) => void)[] = [];
  createInterface({input: core.stdout}).on('line', s => {
    const m = JSON.parse(s);
    if (m.event) {
      event?.(m);
      return;
    }
    pending.shift()?.(s);
  });
  let stopped = '';
  function stop(reason: string) {
    stopped = reason;
    for (const finish of pending.splice(0))
      finish(JSON.stringify({error: reason}));
  }
  core.once('error', e => stop(e.message));
  core.stdin.on('error', e => stop(e.message));
  core.once('close', (code, signal) =>
    stop('Notebook fixture closed (' + (signal || code) + ').'),
  );
  function rpc(op: string, args = {}) {
    return new Promise<any>((resolve, reject) => {
      if (stopped) return reject(Error(stopped));
      pending.push(s => {
        const result = JSON.parse(s);
        if (result.error) reject(Error(result.error));
        else resolve(result.ok);
      });
      core.stdin.write(
        JSON.stringify({operation: op, args, stream: !!event}) + '\n',
      );
    });
  }
  return {core, rpc, folder};
}
