import { useState } from 'react';
import './thread-controls.css';

export type ThreadOperation =
  | {type:'fork'|'archive'|'unarchive'|'compact'}
  | {type:'rename';name:string}
  | {type:'review';target:{type:'uncommittedChanges'}|{type:'baseBranch';branch:string}|{type:'commit';sha:string}|{type:'custom';instructions:string}};

export function ThreadControls({threadId,name,archived=false,disabled=false,archiveBlocked=false,onAction}:{threadId:string;name?:string|null;archived?:boolean;disabled?:boolean;archiveBlocked?:boolean;onAction:(action:ThreadOperation)=>Promise<boolean>}) {
  const [operation,setOperation]=useState('rename');
  const [nextName,setNextName]=useState(name??'');
  const [target,setTarget]=useState('uncommittedChanges');
  const [detail,setDetail]=useState('');
  const [confirmed,setConfirmed]=useState(false);
  const blocked=disabled||(!archived&&operation==='archive'&&archiveBlocked);
  async function submit() {
    if(blocked||invalid)return;
    let action:ThreadOperation;
    if(archived)action={type:'unarchive'};
    else if(operation==='rename')action={type:'rename',name:nextName.trim()};
    else if(operation==='review')action={type:'review',target:target==='baseBranch'?{type:'baseBranch',branch:detail.trim()}:target==='commit'?{type:'commit',sha:detail.trim()}:target==='custom'?{type:'custom',instructions:detail}:{type:'uncommittedChanges'}};
    else action={type:operation as 'fork'|'archive'|'compact'};
    if(await onAction(action))setConfirmed(false);
  }
  const invalid=!archived&&((operation==='rename'&&!nextName.trim())||(operation==='review'&&target!=='uncommittedChanges'&&!detail.trim())||(operation==='archive'&&!confirmed));
  return <details className="thread-controls">
    <summary>Thread actions</summary>
    <form className="thread-control-panel" onSubmit={event=>{event.preventDefault();void submit();}}>
      <p className="thread-control-id">Codex · {threadId}</p>
      {archived?<p>This conversation is archived. Restore it to continue working.</p>:<>
        <label>Action<select aria-label="Thread action" value={operation} disabled={disabled} onChange={event=>{setOperation(event.target.value);setConfirmed(false);}}>
          <option value="rename">Rename</option><option value="fork">Fork</option><option value="archive">Archive</option><option value="compact">Compact context</option><option value="review">Review code</option>
        </select></label>
        {operation==='rename'&&<label>Conversation name<input aria-label="Conversation name" value={nextName} disabled={disabled} maxLength={256} onChange={event=>setNextName(event.target.value)}/></label>}
        {operation==='fork'&&<p>Open a new conversation with this thread’s saved history. Your source tab and unsent draft stay here.</p>}
        {operation==='archive'&&<><p>Move this conversation and any descendants Codex can archive into native archived history. Restore it from Archived.</p>{archiveBlocked&&<p role="status">Finish this project’s turns and approvals before archiving. An uncertain connection also blocks archive.</p>}</>}
        {operation==='compact'&&<p>Ask Codex to compact this conversation’s context. This can use your model allowance. Progress arrives in the conversation; acceptance does not mean completion.</p>}
        {operation==='review'&&<>
          <label>Review target<select aria-label="Review target" value={target} disabled={disabled} onChange={event=>{setTarget(event.target.value);setDetail('');setConfirmed(false);}}><option value="uncommittedChanges">Uncommitted changes</option><option value="baseBranch">Against a base branch</option><option value="commit">A commit</option><option value="custom">Custom instructions</option></select></label>
          {target!=='uncommittedChanges'&&<label>{target==='baseBranch'?'Base branch':target==='commit'?'Commit SHA':'Review instructions'}<textarea aria-label="Review target detail" value={detail} disabled={disabled} maxLength={target==='custom'?65536:target==='commit'?64:512} onChange={event=>{setDetail(event.target.value);setConfirmed(false);}}/></label>}
          <p>Start a Codex review turn in this conversation. It uses the runtime’s permissions and model allowance. Fork first if you want a separate review conversation.</p>
        </>}
        {operation==='archive'&&<label className="thread-control-confirm"><input type="checkbox" checked={confirmed} disabled={blocked} onChange={event=>setConfirmed(event.target.checked)}/> Archive this conversation and eligible descendants.</label>}
      </>}
      <button type="submit" disabled={blocked||invalid}>{disabled?'Waiting for Codex…':archived?'Restore conversation':operation==='rename'?'Save name':operation==='fork'?'Fork into new tab':operation==='archive'?'Archive conversation':operation==='compact'?'Start compaction':'Start review'}</button>
    </form>
  </details>;
}
