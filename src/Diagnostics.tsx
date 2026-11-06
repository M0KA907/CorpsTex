import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { diagnostics, diagnosticsPrompt, diagnosticSummary, type Diagnostic } from './diagnostics';
import './diagnostics.css';

export function Diagnostics({suppressed=false}:{suppressed?:boolean}) {
  const snapshot=useSyncExternalStore(diagnostics.subscribe,diagnostics.getSnapshot,diagnostics.getSnapshot);
  const [open,setOpen]=useState(false);
  const [selected,setSelected]=useState<number|null>(null);
  const [feedback,setFeedback]=useState('');
  const [feedbackId,setFeedbackId]=useState<number|null>(null);
  const dialog=useRef<HTMLDialogElement>(null);
  const prompt=useRef<HTMLTextAreaElement>(null);
  const copying=useRef(false);
  const pending=snapshot.entries.filter(d=>d.significant&&!d.dismissed);
  const popup=pending.at(-1);
  const entry=snapshot.entries.find(d=>d.id===selected)??snapshot.entries.at(-1);
  useEffect(()=>{if(suppressed)setOpen(false);},[suppressed]);
  useEffect(()=>{if(open&&!suppressed)dialog.current?.showModal();else dialog.current?.close();},[open,suppressed]);
  function review(id?:number){setSelected(id??snapshot.entries.at(-1)?.id??null);setFeedback('');setOpen(true);}
  async function copy(item:Diagnostic){
    if(copying.current)return;copying.current=true;setFeedback('');setFeedbackId(item.id);
    try {
      if(!navigator.clipboard?.writeText)throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(diagnosticsPrompt(item));
      setFeedback('Diagnostics prompt copied. Paste it manually into your chosen assistant.');
    } catch {
      setSelected(item.id);setOpen(true);
      setFeedback('Clipboard access failed. Select the prompt below and copy it manually.');
      requestAnimationFrame(()=>{prompt.current?.focus();prompt.current?.select();});
    } finally {copying.current=false;}
  }
  return <>
    <button className="diagnostics-launcher" type="button" aria-haspopup="dialog" aria-expanded={open} onClick={()=>review()} disabled={suppressed}>Diagnostics{pending.length?` (${pending.length})`:''}</button>
    {popup&&!suppressed&&!open&&<aside className="diagnostics-popup" aria-label="Application failure">
      <p role="status"><strong>Application needs attention</strong><br/>{diagnosticSummary(popup)}</p>
      <p>{popup.runtime} · {popup.project} · {popup.count} occurrence{popup.count===1?'':'s'}</p>
      <div className="diagnostics-actions"><button type="button" onClick={()=>void copy(popup)}>Copy Diagnostics Prompt</button><button type="button" onClick={()=>review(popup.id)}>Review details</button><button type="button" onClick={()=>{diagnostics.dismiss(popup.id);setFeedback('');}}>Dismiss</button></div>
      {feedback&&feedbackId===popup.id&&<p role="status">{feedback}</p>}
    </aside>}
    <dialog className="diagnostics-dialog" ref={dialog} aria-labelledby="diagnostics-title" onCancel={event=>{event.preventDefault();setOpen(false);}} onClose={event=>{if(!event.currentTarget.open)setOpen(false);}}>
      <header><h1 id="diagnostics-title">Application diagnostics</h1><button type="button" autoFocus onClick={()=>setOpen(false)} aria-label="Close diagnostics">Close ×</button></header>
      <div className="diagnostics-content">
        <p>Available in this window until reload or exit. Nothing is submitted automatically. Paths, content and raw error messages are omitted.</p>
        {snapshot.omitted>0&&<p role="status">{snapshot.omitted} older distinct failures omitted; only the latest 32 are retained.</p>}
        {snapshot.entries.length===0?<p>No application failures recorded in this window.</p>:<>
          <label>Recorded failure<select value={entry?.id} onChange={event=>{setSelected(Number(event.target.value));setFeedback('');}}>{[...snapshot.entries].reverse().map(d=><option key={d.id} value={d.id}>{d.significant?'Attention':'Recoverable'} · {d.runtime} · {d.project} · {d.operation} · {d.kind} ({d.count})</option>)}</select></label>
          {entry&&<>
            <h2>{diagnosticSummary(entry)}</h2>
            <p>{entry.significant?'Requires attention.':'Recorded without an automatic popup.'} Recovery remains manual; use the existing project and runtime controls.</p>
            <label>Diagnostics prompt<textarea ref={prompt} readOnly rows={14} value={diagnosticsPrompt(entry)} onFocus={event=>event.currentTarget.select()}/></label>
            <div className="diagnostics-actions"><button type="button" onClick={()=>void copy(entry)}>Copy Diagnostics Prompt</button>{entry.significant&&!entry.dismissed&&<button type="button" onClick={()=>diagnostics.dismiss(entry.id)}>Dismiss notification</button>}</div>
          </>}
        </>}
        {feedback&&feedbackId===entry?.id&&<p role="status">{feedback}</p>}
      </div>
    </dialog>
  </>;
}
