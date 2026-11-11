import { FolderBrowser } from '../FolderBrowser';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { authorize, closeFile, dirty, fs, grantEditorAccess, message, notice, openFile, invoke, relocatedPath, revokeEditorAccess, setEditorContext, useEditor } from './state';
type Entry={name:string;path:string;directory:boolean;symlink:boolean};
type Listing={entries:Entry[];truncated:boolean};
const locations=new Map<string,string>();
export function FileExplorer({root:projectRoot,runtime,hidden,onProject,onNewConversation,header,navigation}:{root:string;runtime:string;hidden:boolean;onProject:(root:string)=>void;header?:ReactNode;navigation?:ReactNode;onNewConversation:()=>void}){
  const [picker,setPicker]=useState<'project'|'extra'|null>(null);
  const [extra,setExtra]=useState('');const [grantedRoot,setGrantedRoot]=useState('');
  const root=extra||grantedRoot||projectRoot;
  useEffect(()=>{setExtra('');setGrantedRoot('');},[projectRoot,runtime]);
  const {files}=useEditor();const [directory,setDirectory]=useState('');const [listing,setListing]=useState<Listing>({entries:[],truncated:false});
  const [filter,setFilter]=useState('');const [selected,setSelected]=useState<Entry|null>(null);const [destination,setDestination]=useState('');const [busy,setBusy]=useState(false);const [error,setError]=useState('');
  const binding=useRef('');binding.current=JSON.stringify([projectRoot,runtime,root,directory]);const refreshBusy=useRef(false);
  useEffect(()=>{setDirectory(locations.get(root)??'');setSelected(null);setListing({entries:[],truncated:false});setError('');},[root,projectRoot,runtime]);
  async function refresh(){
    if(!root||refreshBusy.current)return;refreshBusy.current=true;const current=binding.current;
    try{await setEditorContext(projectRoot,runtime);const canonical=await authorize(root);const result=await fs<Listing>(canonical,{type:'list',path:directory});if(current===binding.current){setListing(result);setError('');}}
    catch(e){if(current===binding.current){setListing({entries:[],truncated:false});setError(message(e));}}finally{refreshBusy.current=false;}
  }
  useEffect(()=>{if(hidden)return;void refresh();const timer=setInterval(()=>void refresh(),2500);return()=>clearInterval(timer);},[root,runtime,directory,hidden]);
  async function grant(path:string,additional=false){
    if(busy)return;setBusy(true);setError('');const current=binding.current;
    try{await setEditorContext(projectRoot,runtime);const canonical=await grantEditorAccess(path);if(current!==binding.current)return;if(additional)setExtra(canonical);else setGrantedRoot(canonical);setPicker(null);await refresh();}
    catch(e){if(current===binding.current)setError(message(e));}finally{setBusy(false);}
  }
  function navigate(path:string){locations.set(root,path);setDirectory(path);setSelected(null);setDestination(path?path+'/':'');}
  async function operation(type:'create'|'directory'|'rename'|'duplicate'|'delete'){
    if(busy)return;
    const originalRoot=root;const original=selected;const target=destination.trim();
    if((type==='delete'||type==='rename'||type==='duplicate')&&!original)return;
    const affected=original?files.filter(f=>relocatedPath(f,root,original.path,target,original.directory)!==null):[];
    if((type==='delete'||type==='rename')&&affected.some(f=>dirty(f)||f.conflict||f.saving)){setError('Save or close the unsaved file before moving or deleting it.');return;}
    setBusy(true);setError('');
    try{
      const canonical=await authorize(root);
      if(type==='delete'){
        const inspected=await fs<{revision:string|null;bytes:number;directory:boolean}>(canonical,{type:'inspect',path:original!.path});
        if(!window.confirm(`Permanently delete ${original!.path}${inspected.directory?' (must be empty)':` (${inspected.bytes} bytes)`}? This cannot be undone.`))return;
        const expected=inspected.revision??undefined;
        await fs(canonical,{type:'delete',path:original!.path,expected});for(const file of affected)closeFile(file.id);
      }else if(type==='create'||type==='directory'){
        await fs(canonical,{type:'create',path:target,directory:type==='directory'});if(type==='create')await openFile(canonical,target,true);
      }else{
        await fs(canonical,{type,path:original!.path,destination:target});
        if(type==='rename')for(const file of affected){closeFile(file.id);await openFile(canonical,relocatedPath(file,root,original!.path,target,original!.directory)!,true,file.cursor);}
      }
      if(originalRoot===root){setSelected(null);await refresh();}notice(type==='delete'?'Deleted '+original!.path:'File operation completed.');
    }catch(e){setError(message(e));}finally{setBusy(false);}
  }
  return <aside className="file-explorer projects" hidden={hidden} aria-label="Project files">
    <div className="sidebar-header">{header}{navigation}</div>
    <div className="thread-actions"><button type="button" onClick={onNewConversation}>New conversation</button><button type="button" disabled={busy||!root} onClick={()=>void refresh()} aria-label="Refresh files" title="Refresh files"><svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M20 7v5h-5M4 17v-5h5"/><path d="M6.1 7a7 7 0 0 1 11.5-1L20 9M4 15l2.4 3A7 7 0 0 0 18 17"/></svg></button></div>
    <div className="panel-heading"><h2>Files</h2></div><button type="button" onClick={()=>setPicker('project')}>Open project folder for editing</button>
    {picker&&<FolderBrowser initialPath={root} invoke={invoke} errorText={message} onClose={()=>setPicker(null)} onSelect={path=>{if(picker==='extra'){void grant(path,true);}else{onProject(path);setPicker(null);}}}/>}
    <button type="button" disabled={busy||!root} onClick={()=>void grant(root)}>Grant editor access</button>
    <button type="button" disabled={busy||!projectRoot} onClick={()=>setPicker('extra')}>Grant an additional folder…</button>
    {extra&&<button type="button" onClick={()=>setExtra('')}>Back to project files</button>}
    <button type="button" disabled={busy||!projectRoot} onClick={()=>{void revokeEditorAccess().then(()=>{setListing({entries:[],truncated:false});notice('Editor access revoked. Open buffers are retained.');}).catch(e=>setError(message(e)));}}>Revoke editor access</button>
    <p>Access applies to this window, project and runtime until you switch context or close the app. Open buffers are retained.</p>
    <p className="explorer-root" title={root}>{root||'Choose a project folder'}</p>
    <div className="explorer-breadcrumb"><button type="button" disabled={!directory} onClick={()=>navigate(directory.split('/').slice(0,-1).join('/'))} aria-label="Parent directory">↑</button><button type="button" onClick={()=>navigate('')}>Project</button><span title={directory}>{directory||'/'}</span></div>
    <input type="search" aria-label="Filter files" placeholder="Filter this folder…" value={filter} onChange={e=>setFilter(e.target.value)}/>
    <nav className="explorer-list" aria-label="Project directory contents">{listing.entries.filter(e=>e.name.toLowerCase().includes(filter.toLowerCase())).map(entry=><div className="explorer-entry" key={entry.path} data-selected={selected?.path===entry.path}>
      <button type="button" disabled={entry.symlink||entry.name==='.git'||entry.name.startsWith('.corpstex-save-')} title={entry.symlink?'Symlinks are protected':entry.path} onClick={()=>{setSelected(entry);setDestination(entry.path);if(entry.directory)navigate(entry.path);else void openFile(root,entry.path);}} onDoubleClick={()=>{if(!entry.directory)void openFile(root,entry.path,true);}}><span aria-hidden="true">{entry.symlink?'↗':entry.directory?'▸':'·'}</span><span>{entry.name}</span>{files.some(f=>f.root===root&&f.path===entry.path&&dirty(f))&&<span aria-label="Unsaved">●</span>}</button>
      {entry.directory&&!entry.symlink&&<button type="button" className="explorer-select" aria-label={`Manage folder ${entry.name}`} onClick={()=>{setSelected(entry);setDestination(entry.path);}}>⋯</button>}
    </div>)}</nav>
    {listing.truncated&&<p role="status">First 5,000 entries shown.</p>}
    <form className="file-actions" onSubmit={e=>{e.preventDefault();void operation('create');}}>
      <p>{selected?`Selected: ${selected.path}`:'Create a file or folder'}</p>
      <label>Project-relative destination<input required aria-label="File destination" value={destination} onChange={e=>setDestination(e.target.value)} placeholder="src/example.ts" disabled={busy}/></label>
      <div><button disabled={busy||!root||!destination.trim()} type="submit">New file</button><button disabled={busy||!root||!destination.trim()} type="button" onClick={()=>void operation('directory')}>New folder</button></div>
      {selected&&<div><button disabled={busy||!destination.trim()||selected.path===destination} type="button" onClick={()=>void operation('rename')}>Rename / move</button><button disabled={busy||!destination.trim()||selected.directory||selected.path===destination} type="button" onClick={()=>void operation('duplicate')}>Duplicate</button><button type="button" disabled={busy} onClick={()=>void operation('delete')}>Delete…</button></div>}
    </form>
    {busy&&<p role="status">Working…</p>}{error&&<p role="alert">{error}</p>}
  </aside>;
}
