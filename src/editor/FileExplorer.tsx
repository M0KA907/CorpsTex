import { WorkspaceIcon } from '../WorkspaceIcon';
import { uiScale } from '../AppZoom';
import { WorkspaceDrawer } from '../WorkspaceDrawer';
import { FolderBrowser } from '../FolderBrowser';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { authorize, dirty, fs, grantEditorAccess, manageFiles, message, notice, openFile, invoke, revokeEditorAccess, setEditorContext, useEditor } from './state';
type Entry={name:string;path:string;directory:boolean;symlink:boolean};
type Listing={entries:Entry[];truncated:boolean};
const locations=new Map<string,string>();
export function FileExplorer({root:projectRoot,runtime,hidden,onProject,onNewConversation,header,navigation}:{root:string;runtime:string;hidden:boolean;onProject:(root:string)=>void;header?:ReactNode;navigation?:ReactNode;onNewConversation:()=>void}){
  const [menuAnchor,setMenuAnchor]=useState({x:0,y:0});
  const [accessOpen,setAccessOpen]=useState(false);
  const [action,setAction]=useState<'menu'|'create'|'directory'|'rename'|'duplicate'|null>(null);
  const [picker,setPicker]=useState<'project'|'extra'|null>(null);
  const [roots,setRoots]=useState<string[]>([]);
  const [extra,setExtra]=useState('');const [grantedRoot,setGrantedRoot]=useState('');
  const root=extra||grantedRoot||projectRoot;
  useEffect(()=>{setExtra('');setGrantedRoot('');setRoots([]);setAccessOpen(false);setAction(null);setPicker(null);},[projectRoot,runtime]);
  const {files,active}=useEditor();const [directory,setDirectory]=useState('');const [listing,setListing]=useState<Listing>({entries:[],truncated:false});
  const [filter,setFilter]=useState('');const [selected,setSelected]=useState<Entry|null>(null);const [destination,setDestination]=useState('');const [busy,setBusy]=useState(false);const [error,setError]=useState('');
  const binding=useRef('');binding.current=JSON.stringify([projectRoot,runtime,root,directory]);const refreshBusy=useRef(false);
  useEffect(()=>{setDirectory(locations.get(root)??'');setSelected(null);setAction(null);setListing({entries:[],truncated:false});setError('');},[root,projectRoot,runtime]);
  async function refresh(){
    if(!root||refreshBusy.current)return;refreshBusy.current=true;const current=binding.current;
    try{await setEditorContext(projectRoot,runtime);const access=await fs<{roots:string[]}>(projectRoot,{type:'roots'});if(current!==binding.current)return;setRoots(access.roots);const canonical=await authorize(root);const result=await fs<Listing>(canonical,{type:'list',path:directory});if(current===binding.current){setListing(result);setError('');}}
    catch(e){if(current===binding.current){setListing({entries:[],truncated:false});setError(message(e));}}finally{refreshBusy.current=false;}
  }
  useEffect(()=>{if(hidden)return;void refresh();const timer=setInterval(()=>void refresh(),2500);return()=>clearInterval(timer);},[root,runtime,directory,hidden]);
  async function grant(path:string,additional=false){
    if(busy)return;setBusy(true);setError('');const current=binding.current;
    try{await setEditorContext(projectRoot,runtime);const canonical=await grantEditorAccess(path);if(current!==binding.current)return;setRoots(old=>[...new Set([...old,canonical])].sort());if(additional)setExtra(canonical);else setGrantedRoot(canonical);setPicker(null);await refresh();}
    catch(e){if(current===binding.current)setError(message(e));}finally{setBusy(false);}
  }
  function navigate(path:string){locations.set(root,path);setDirectory(path);setSelected(null);setDestination(path?path+'/':'');}
  async function operation(type:'create'|'directory'|'rename'|'duplicate'|'delete'){
    if(busy)return;
    const originalRoot=root;const original=selected;const target=destination.trim();
    if((type==='delete'||type==='rename'||type==='duplicate')&&!original)return;
    setBusy(true);setError('');
    try{
      if(type==='delete'||type==='rename'){
        if(!await manageFiles(root,original!.path,target,original!.directory,type))return;
      }else if(type==='create'||type==='directory'){
        const canonical=await authorize(root);
        await fs(canonical,{type:'create',path:target,directory:type==='directory'});if(type==='create')await openFile(canonical,target,true);
      }else{
        const canonical=await authorize(root);
        await fs(canonical,{type,path:original!.path,destination:target});
      }
      if(originalRoot===root){setSelected(null);setAction(null);await refresh();}notice(type==='delete'?'Deleted '+original!.path:type==='duplicate'?'Duplicated saved files on disk. Unsaved editor changes were not copied.':'File operation completed.');
    }catch(e){setError(message(e));}finally{setBusy(false);}
  }
  function entryMenu(entry:Entry,element:HTMLElement){
    const bounds=element.getBoundingClientRect();
    const scale=uiScale();setMenuAnchor({x:bounds.right/scale+8,y:bounds.top/scale});setSelected(entry);setDestination(entry.path);setAction('menu');
  }
  const granted=roots.includes(root);
  const activeFile=files.find(file=>file.id===active);
  const folderName=root.split(/[\\/]/).filter(Boolean).at(-1)||'Project';
  return <aside className="file-explorer projects" hidden={hidden} inert={hidden} aria-hidden={hidden} data-root={root} aria-label="Project files">
    <div className="sidebar-header">{header}{navigation}</div>
    <div className="thread-actions"><button type="button" onClick={onNewConversation}><WorkspaceIcon name="plus"/>New conversation</button><button type="button" disabled={busy||!root} onClick={()=>void refresh()} aria-label="Refresh files" title="Refresh files"><WorkspaceIcon name="refresh"/></button></div>
    <div className="explorer-content">
      <div className="explorer-breadcrumb">
        <button type="button" disabled={!directory} onClick={()=>navigate(directory.split('/').slice(0,-1).join('/'))} aria-label="Parent directory" title="Parent directory"><WorkspaceIcon name="arrow"/></button>
        <button type="button" onClick={()=>navigate('')} title={root}><WorkspaceIcon name="folder"/><span>{folderName}</span></button>
        {directory&&<span title={directory}>/ {directory}</span>}
      </div>
      <div className="explorer-toolbar">
        <input type="search" aria-label="Filter files" placeholder="Find a file…" value={filter} onChange={e=>setFilter(e.target.value)}/>
        <button type="button" disabled={busy||!root} onClick={()=>{setDestination(directory?directory+'/':'');setAction('create');}} title="New file" aria-label="New file"><WorkspaceIcon name="file"/><span aria-hidden="true">+</span></button>
        <button type="button" disabled={busy||!root} onClick={()=>{setDestination(directory?directory+'/':'');setAction('directory');}} title="New folder" aria-label="New folder"><WorkspaceIcon name="folder"/><span aria-hidden="true">+</span></button>
      </div>
      <nav className="explorer-list" aria-label="Project directory contents">{listing.entries.filter(e=>e.name.toLowerCase().includes(filter.toLowerCase())).map(entry=>{
        const protectedEntry=entry.symlink||entry.name==='.git'||entry.name.startsWith('.corpstex-save-');
        return <div className="explorer-entry" key={entry.path} data-selected={action?selected?.path===entry.path:activeFile?.root===root&&activeFile.path===entry.path} onContextMenu={event=>{if(protectedEntry)return;event.preventDefault();entryMenu(entry,event.currentTarget);}}>
          <button type="button" disabled={protectedEntry} title={entry.symlink?'Symlinks are protected':entry.path} onClick={()=>{setSelected(entry);setDestination(entry.path);if(entry.directory)navigate(entry.path);else void openFile(root,entry.path);}} onDoubleClick={()=>{if(!entry.directory)void openFile(root,entry.path,true);}}><WorkspaceIcon name={entry.directory?'folder':'file'}/><span>{entry.name}</span>{files.some(f=>f.root===root&&f.path===entry.path&&dirty(f))&&<span aria-label="Unsaved">●</span>}</button>
          {!protectedEntry&&<button type="button" className="explorer-select" aria-label={`Manage ${entry.directory?'folder':'file'} ${entry.name}`} title={`Actions for ${entry.name}`} aria-haspopup="dialog" onClick={event=>entryMenu(entry,event.currentTarget)}>⋯</button>}
        </div>;
      })}</nav>
      {!listing.entries.length&&!busy&&granted&&!error&&<p className="explorer-empty">Empty folder</p>}
      {listing.truncated&&<p role="status">First 5,000 entries</p>}
      {busy&&<p role="status">Working…</p>}{error&&(granted||error!=='Editor access required.')&&<p role="alert">{error}</p>}
    </div>
    <div className="editor-access-row" data-granted={granted}><WorkspaceIcon name="access"/><span>{granted?'Editor access':'Access required'}</span><button type="button" disabled={busy||!root} aria-label={granted?'Manage editor access':'Grant editor access'} onClick={()=>granted?setAccessOpen(true):void grant(root)} aria-haspopup="dialog">{granted?'Manage':'Grant access'}</button></div>
    {accessOpen&&<WorkspaceDrawer title="Editor access" onClose={()=>setAccessOpen(false)}>
      <div className="access-folder"><WorkspaceIcon name="folder" size={22}/><strong>{folderName}</strong><span className="drawer-path">{root||'No project selected'}</span></div>
      <p className="access-scope">This window · {runtime} · expires on project switch or exit</p>
      <div className="access-actions">
        <button type="button" className="access-primary" disabled={busy||!root||granted} onClick={()=>void grant(root)}>{granted?'Access granted':'Grant editor access'}</button>
        <button type="button" aria-label="Open project folder for editing" onClick={()=>{setAccessOpen(false);setPicker('project');}}>Choose project…</button>
        <button type="button" aria-label="Grant an additional folder…" disabled={busy||!projectRoot} onClick={()=>{setAccessOpen(false);setPicker('extra');}}>Add folder…</button>
      </div>
      {roots.length>0&&<label className="access-folders">Folders<select aria-label="Authorized editor folders" value={roots.includes(root)?root:''} onChange={e=>{setExtra(e.target.value===(grantedRoot||projectRoot)?'':e.target.value);setSelected(null);setDestination('');setAccessOpen(false);}}><option value="" disabled>Choose a folder</option>{roots.map(path=><option key={path} value={path}>{path}</option>)}</select></label>}
      {extra&&<button type="button" onClick={()=>{setExtra('');setAccessOpen(false);}}>Back to project files</button>}
      <button type="button" className="access-revoke" aria-label="Revoke editor access" disabled={busy||!roots.length} onClick={()=>{void revokeEditorAccess().then(()=>{setRoots([]);setExtra('');setGrantedRoot('');setListing({entries:[],truncated:false});setAccessOpen(false);notice('Editor access revoked.');}).catch(e=>setError(message(e)));}}>Revoke access</button>
      {error&&<p role="alert">{error}</p>}
    </WorkspaceDrawer>}
    {picker&&<FolderBrowser initialPath={root} invoke={invoke} errorText={message} onClose={()=>setPicker(null)} onSelect={path=>{if(picker==='extra'){void grant(path,true);}else{onProject(path);setPicker(null);}}}/>}
    {action&&<WorkspaceDrawer title={action==='menu'?selected?.name||'File actions':action==='create'?'New file':action==='directory'?'New folder':action==='rename'?'Rename / move':'Duplicate'} anchor={action==='menu'?menuAnchor:undefined} onClose={()=>{if(!busy)setAction(null);}}>
      <form className="file-actions" onSubmit={e=>{e.preventDefault();if(action!=='menu')void operation(action);}}>
        {action==='menu'?<div>
          <button disabled={busy} type="button" onClick={()=>setAction('rename')}>Rename / move</button>
          <button disabled={busy} type="button" onClick={()=>setAction('duplicate')}>Duplicate</button>
          <button disabled={busy} type="button" onClick={()=>void operation('delete')}>Delete…</button>
        </div>:<>
          <p className="drawer-path">{selected&&action!=='create'&&action!=='directory'?selected.path:directory||folderName}</p>
          <label>Destination<input key={action} autoFocus required aria-label="File destination" value={destination} onChange={e=>setDestination(e.target.value)} placeholder="src/example.ts" disabled={busy}/></label>
          {action==='duplicate'&&<p>Copies saved files on disk.</p>}
          {selected?.directory&&<details><summary>Folder limits</summary><p>5,000 entries · 32 levels · 32 MiB per file · 64 MiB total. No symlinks or protected metadata. Only empty folders can be deleted.</p></details>}
          <button disabled={busy||!root||!destination.trim()||((action==='rename'||action==='duplicate')&&selected?.path===destination.trim())} type="submit">{action==='create'?'New file':action==='directory'?'New folder':action==='rename'?'Rename / move':'Duplicate'}</button>
        </>}
        {busy&&<p role="status">Working…</p>}{error&&<p role="alert">{error}</p>}
      </form>
    </WorkspaceDrawer>}
  </aside>;
}
