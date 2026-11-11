import { useEffect, useRef } from 'react';
import { pruneEditorViews } from './EditorSurface';
import { closeFile, dirty, discardFile, discardEditorRecovery, getEditor, invoke, message, notice, persistEditorSession, restoreEditorSession, saveFile, syncFile, useEditor } from './state';
export function EditorChrome(){
  const state=useEditor();const used=useRef(false);const dirtyNow=state.files.some(f=>dirty(f)||Boolean(f.conflict));
  useEffect(()=>{void invoke('editor_dirty',{dirty:dirtyNow}).catch(e=>{if(dirtyNow)notice('Close protection could not reach the desktop: '+message(e));});},[dirtyNow]);
  useEffect(()=>{
    let polling=false;let gone=false;const lastSync=new Map<string,number>();
    const timer=setInterval(()=>{
      if(polling)return;polling=true;
      void (async()=>{
        for(const file of getEditor().files){if(gone)return;if(file.large&&Date.now()-(lastSync.get(file.id)??0)<10000)continue;await syncFile(file.id);lastSync.set(file.id,Date.now());}
        const files=getEditor().files;
        if(await invoke<boolean>('editor_exit_pending')){
          if(await invoke<boolean>('editor_install_busy')){notice('A language server installation is running. Wait for it to finish before exiting.');await invoke('editor_resolve_exit',{discard:false});return;}
          const discard=window.confirm('Discard all unsaved editor buffers and continue exiting CorpsTex? There is no durable recovery. Cancel keeps the application open.');
          if(discard)for(const file of files)if(dirty(file)||file.conflict)discardFile(file.id);
          await invoke('editor_resolve_exit',{discard});
        }
      })().catch(e=>notice(message(e))).finally(()=>{polling=false;});
    },1500);
    const unload=(event:BeforeUnloadEvent)=>{persistEditorSession();if(getEditor().files.some(f=>dirty(f)||f.conflict)){event.preventDefault();event.returnValue='';}};
    const keys=(event:KeyboardEvent)=>{
      if(!(event.ctrlKey||event.metaKey)||event.altKey)return;
      const active=getEditor().active;if(!active)return;
      if(event.key.toLowerCase()==='s'){event.preventDefault();void saveFile(active);}
      if(event.key.toLowerCase()==='w'){event.preventDefault();closeFile(active);}
    };
    window.addEventListener('beforeunload',unload);window.addEventListener('keydown',keys);
    return()=>{gone=true;clearInterval(timer);window.removeEventListener('beforeunload',unload);window.removeEventListener('keydown',keys);};
  },[]);
  useEffect(()=>{if(!used.current&&!state.files.length)return;used.current=true;void import('./monaco').then(m=>{const ids=new Set(getEditor().files.map(f=>f.id));m.disposeClosedModels(ids);pruneEditorViews(ids);});},[state.files.map(f=>f.id).join(',')]);
  return <>{state.recovery&&<aside className="editor-recovery editor-warning" aria-label="Previous editor session">
    <strong>Previous editor session available</strong>
    <p>{state.recovery.tabs.length} tab references. {state.recovery.tabs.filter(t=>t.unsaved).length} had unsaved edits whose text is unrecoverable. Restoration requires confirmation; folder access must be granted again.</p>
    <button type="button" onClick={()=>{if(restoreEditorSession(state.recovery!))requestAnimationFrame(()=>document.querySelector<HTMLButtonElement>('.file-tab [aria-selected="true"]')?.focus());}}>Restore editor session…</button>
    <button type="button" onClick={()=>{if(window.confirm('Forget the previous editor tab references? Current open buffers will be retained.'))discardEditorRecovery();}}>Forget previous session…</button>
  </aside>}{state.notice&&<div className="editor-notice" role="status"><span>{state.notice}</span><button type="button" aria-label="Dismiss editor notice" onClick={()=>notice('')}>×</button></div>}
  </>;
}
