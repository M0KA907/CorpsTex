import type { CSSProperties } from 'react';
export const activityPresets = [
  ['orbit','Orbit'], ['pulse','Pulse'], ['wave','Wave'], ['dots','Dots'], ['scan','Scan'],
  ['steps','Steps'], ['ribbon','Ribbon'], ['beacon','Beacon'], ['matrix','Matrix'], ['minimal','Minimal'],
] as const;
export type ActivityPreset = typeof activityPresets[number][0];
type ActivityItem = {type:string; status?:string; clientActivityStatus?:string};
type ActivityTurn = {status:string; items:ActivityItem[]};
export function activityState(ready:boolean, selected:boolean, turns:ActivityTurn[], waiting:boolean, unavailable=false) {
  if (unavailable) return {label:'Status unavailable', active:false};
  if (!ready) return {label:'Disconnected', active:false};
  if (!selected) return {label:'No conversation open', active:false};
  if (waiting) return {label:'Waiting for your response', active:false};
  const active = [...turns].reverse().find(turn=>turn.status==='inProgress');
  if (!active) {
    const status = turns.at(-1)?.status;
    return {label:status==='failed'?'Turn failed':status==='interrupted'?'Stopped':'Ready',active:false};
  }
  const item=[...active.items].reverse().find(item=>item.clientActivityStatus==='inProgress');
  const labels:Record<string,string>={reasoning:'Thinking',agentMessage:'Responding',commandExecution:'Running a command',fileChange:'Editing files',mcpToolCall:'Using a tool',webSearch:'Searching the web',collabAgentToolCall:'Coordinating agents'};
  return {label:item ? labels[item.type] ?? 'Working' : 'Working',active:true};
}
export function ActivityBar({preset,label,active,compact=false,preview=false}:{preset:ActivityPreset;label:string;active:boolean;compact?:boolean;preview?:boolean}) {
  return <div className="activity-bar" data-preset={preset} data-active={active} data-compact={compact} role={preview?undefined:'status'} aria-live={preview?'off':'polite'}>
    <span className="activity-visual" aria-hidden="true">{Array.from({length:12},(_,i)=><i key={i} style={{'--i':i} as CSSProperties}/>)}</span>
    <span className="activity-label">{label}</span>
  </div>;
}
