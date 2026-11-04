import {useEffect,useMemo,useRef,useState} from 'react';
type Entry={name:string;path:string;isDir:boolean;size:number|null;modified:number|null};
type Listing={path:string;parent:string|null;home:string|null;roots:string[];entries:Entry[];truncated:boolean;skipped:number};
type Invoke=<T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
export function formatFileSize(size:number|null) {if(size==null)return '—';if(size<1024)return `${size} B`;if(size<1024*1024)return `${(size/1024).toFixed(1)} KB`;return `${(size/1024/1024).toFixed(1)} MB`;}
export function FolderBrowser({initialPath,onClose,onSelect,invoke,errorText}:{initialPath:string;onClose:()=>void;onSelect:(path:string)=>void;invoke:Invoke;errorText:(error:unknown)=>string}) {
  const dialog=useRef<HTMLDialogElement>(null);
  const [listing,setListing]=useState<Listing|null>(null);
  const [path,setPath]=useState(initialPath);
  const [hidden,setHidden]=useState(false);
  const [query,setQuery]=useState('');
  const [sort,setSort]=useState('name');
  const [busy,setBusy]=useState(false);
  const [failure,setFailure]=useState('');
  const latest=useRef(0);
  async function open(next:string|null,showHidden=hidden) {
    const id=++latest.current;setBusy(true);setFailure('');
    try {const result=await invoke<Listing>('project_browser',{path:next,showHidden});if(id!==latest.current)return;setListing(result);setPath(result.path);setQuery('');}
    catch(error){if(id===latest.current)setFailure(errorText(error));}
    finally{if(id===latest.current)setBusy(false);}
  }
  useEffect(()=>{const node=dialog.current;node?.showModal();void open(initialPath||null);return()=>{latest.current++;node?.close();};},[]);
  const entries=useMemo(()=>[...(listing?.entries??[])].filter(entry=>entry.name.toLocaleLowerCase().includes(query.toLocaleLowerCase())).sort((a,b)=>Number(b.isDir)-Number(a.isDir)||(sort==='modified'?(b.modified??0)-(a.modified??0):sort==='size'?(b.size??0)-(a.size??0):a.name.localeCompare(b.name))),[listing,query,sort]);
  return <dialog className="settings-dialog folder-dialog" ref={dialog} aria-labelledby="folder-title" onCancel={event=>{event.preventDefault();onClose();}} onClose={event=>{if(!event.currentTarget.open)onClose();}}>
    <header className="settings-header"><h1 id="folder-title">Choose project folder</h1><button type="button" autoFocus onClick={onClose}>Close</button></header>
    <div className="folder-toolbar"><button type="button" disabled={!listing?.parent||busy} onClick={()=>void open(listing!.parent)}>↑ Up</button><button type="button" disabled={busy} onClick={()=>void open(listing?.home??null)}>Home</button>{listing?.roots.map(root=><button type="button" key={root} disabled={busy} onClick={()=>void open(root)}>{root}</button>)}</div>
    <form className="folder-path" onSubmit={event=>{event.preventDefault();void open(path);}}><label htmlFor="browse-path" className="visually-hidden">Folder path</label><input id="browse-path" value={path} onChange={event=>setPath(event.target.value)} spellCheck={false} autoComplete="off"/><button type="submit" disabled={busy}>Go</button></form>
    <div className="folder-filters"><input aria-label="Filter files and folders" placeholder="Filter this folder…" value={query} onChange={event=>setQuery(event.target.value)}/><select aria-label="Sort directory" value={sort} onChange={event=>setSort(event.target.value)}><option value="name">Name</option><option value="modified">Modified</option><option value="size">Size</option></select><label><input type="checkbox" checked={hidden} disabled={busy} onChange={event=>{setHidden(event.target.checked);void open(listing?.path??null,event.target.checked);}}/>Hidden</label></div>
    {failure&&<p className="folder-feedback" role="alert">{failure}</p>}
    {busy&&<p className="folder-feedback" role="status">Loading folder…</p>}
    <div className="folder-list" aria-busy={busy}><table><thead><tr><th scope="col">Name</th><th scope="col">Kind</th><th scope="col">Size</th><th scope="col">Modified</th></tr></thead><tbody>{entries.map(entry=><tr key={entry.path}><td>{entry.isDir?<button type="button" disabled={busy} onClick={()=>void open(entry.path)} title={entry.path}><span aria-hidden="true">▱</span>{entry.name}<span aria-hidden="true">›</span></button>:<span className="folder-file" title={entry.path}><span aria-hidden="true">·</span>{entry.name}</span>}</td><td>{entry.isDir?'Folder':entry.name.includes('.')?entry.name.split('.').at(-1)?.toUpperCase():'File'}</td><td>{formatFileSize(entry.size)}</td><td>{entry.modified?new Date(entry.modified*1000).toLocaleDateString(): '—'}</td></tr>)}</tbody></table>{listing&&!entries.length&&!busy&&<p className="folder-feedback">{query?'No matches.':'This folder is empty.'}</p>}</div>
    <footer className="folder-footer"><div><strong>{listing?.path??'Select a folder'}</strong><small>{entries.length} items{listing?.truncated?' · First 5,000 entries only':''}{listing?.skipped?` · ${listing.skipped} unavailable`:''}</small></div><button type="button" disabled={!listing||busy} onClick={()=>onSelect(listing!.path)}>Use this folder</button></footer>
  </dialog>;
}
