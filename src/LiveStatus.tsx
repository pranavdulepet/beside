import {BesideMark, type Point} from './Ink';

export type ThinkingPhase =
  | 'idle'
  | 'capturing'
  | 'memory'
  | 'web'
  | 'pointing'
  | 'connection'
  | 'thinking'
  | 'streaming';

const labels: Record<ThinkingPhase, string> = {
  idle: '', capturing: 'Reading screen', memory: 'Finding a passage',
  web: 'Searching the web', pointing: 'Finding the control',
  connection: 'Reading the connection', thinking: 'Thinking', streaming: 'Writing',
};

export function LiveStatus({phase, mark}: {phase: ThinkingPhase; mark?: Point[]}) {
  if (phase === 'idle') return null;
  return (
    <div className="live-inline" role="status" aria-atomic="true">
      <span>{labels[phase]}</span>
      <BesideMark size={17} points={mark} animated />
    </div>
  );
}
