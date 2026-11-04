import { useEffect, useRef } from 'react';
import { pruneEditorViews } from './EditorSurface';
import { EditorSettings } from './EditorSettings';
import { closeFile, dirty, discardFile, getEditor, invoke, message, notice, saveFile, showEditorSettings, syncFile, useEditor } from './state';
export function EditorChrome({root}:{root:string}){
  const state=useEditor();const used=useRef(false);const dialog=useRef<HTMLDialogElement>(null);const dirtyNow=state.files.some(f=>dirty(f)||Boolean(f.conflict));
  useEffect(()=>{if(state.settingsOpen)dialog.current?.showModal();else dialog.current?.close();},[state.settingsOpen]);
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
    const unload=(event:BeforeUnloadEvent)=>{if(getEditor().files.some(f=>dirty(f)||f.conflict)){event.preventDefault();event.returnValue='';}};
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
  return <>{state.notice&&<div className="editor-notice" role="status"><span>{state.notice}</span><button type="button" aria-label="Dismiss editor notice" onClick={()=>notice('')}>×</button></div>}
    {state.settingsOpen&&<dialog className="settings-dialog editor-settings-dialog" ref={dialog} aria-label="Editor settings" onCancel={event=>{event.preventDefault();showEditorSettings(false);}}><header className="settings-header"><h1>Settings · Editor</h1><button type="button" autoFocus onClick={()=>showEditorSettings(false)}>Close ×</button></header><div className="settings-body"><EditorSettings root={root}/></div></dialog>}
  </>;
}
