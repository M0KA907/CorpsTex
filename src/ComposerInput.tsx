import { useEffect, useRef, useState, type ReactNode } from 'react';
import { readImageFiles, type ImageAttachment } from './Media';
export type Skill = { name:string; path:string; description:string; enabled:boolean; scope:string; interface?:{displayName?:string;shortDescription?:string}; shortDescription?:string };
type Invoke = <T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
export const composerCommands = [
  {id:'skills',label:'Browse skills',description:'Attach a project skill'},
  {id:'model',label:'Choose model',description:'Set the next turn’s model'},
  {id:'reasoning',label:'Reasoning effort',description:'Set reasoning effort'},
  {id:'new',label:'New conversation',description:'Start a separate conversation'},
  {id:'resume',label:'Find a conversation',description:'Search conversations'},
  {id:'stop',label:'Stop turn',description:'Interrupt the current turn'},
  {id:'plan',label:'Plan mode',description:'Not integrated yet',unavailable:true},
  {id:'review',label:'Code review',description:'Open thread actions to choose a review target'},
  {id:'compact',label:'Compact context',description:'Open thread actions to request compaction'},
];
export function composerToken(value:string) {
  if (/^\/[^\s]*$/.test(value)) return {kind:'commands' as const,query:value.slice(1),start:0};
  const match=/(?:^|\s)\$([^\s]*)$/.exec(value);
  return match ? {kind:'skills' as const,query:match[1],start:value.length-match[1].length-1} : null;
}
export function referencedSkills(value:string, skills:Skill[]) {
  const names=new Set(value.split(/\s+/).map(token=>token.replace(/[.,;!?)]*$/,'')));
  return skills.filter(skill=>names.has(`$${skill.name}`)).map(({name,path})=>({name,path}));
}
export function ComposerInput({idPrefix = '',value,onChange,onSend,onCommand,skills,onSkills,images,onImages,onReading,project,disabled,invoke,errorText,selected,trailing,dockActions}:{idPrefix?:string;onReading:(reading:boolean)=>void;images:ImageAttachment[];onImages:(images:ImageAttachment[])=>void;value:string;onChange:(value:string)=>void;onSend:()=>void;onCommand:(id:string)=>void;skills:Skill[];onSkills:(skills:Skill[])=>void;project:{runtime?:'codex'|'opencode';root:string;generation:number;status:string}|null;disabled:boolean;invoke:Invoke;errorText:(error:unknown)=>string;selected:boolean;trailing?:ReactNode;dockActions?:ReactNode}) {
  const fileInput=useRef<HTMLInputElement>(null);
  const imagesRef=useRef(images); imagesRef.current=images;
  const readingRef=useRef(false);
  const [reading,setReading]=useState(false);
  const [imageError,setImageError]=useState('');
  const mounted=useRef(true);
  useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;onReading(false);};},[]);
  async function attach(files:File[]) {
    if(disabled||readingRef.current)return;
    readingRef.current=true;onReading(true);
    setReading(true);setImageError('');
    try {
      const added=await readImageFiles(files);
      if(!mounted.current)return;
      const next=[...imagesRef.current,...added];
      if(next.length>8||next.reduce((size,image)=>size+image.url.length,0)>8*1024*1024)throw new Error('Attach up to 8 images totaling 6 MiB or less.');
      onImages(next);
    } catch(error) {if(mounted.current)setImageError(error instanceof Error?error.message:'Could not attach images.');}
    finally {readingRef.current=false;if(mounted.current){setReading(false);onReading(false);}}
  }
  const textarea=useRef<HTMLTextAreaElement>(null);
  function resizeInput() {
    const node=textarea.current;if(!node)return;
    node.style.height='auto';node.style.height=`${node.scrollHeight}px`;
  }
  useEffect(resizeInput,[value,selected]);
  useEffect(()=>{
    const node=textarea.current;if(!node)return;let width=node.clientWidth;
    const observer=new ResizeObserver(()=>{if(node.clientWidth!==width){width=node.clientWidth;resizeInput();}});
    observer.observe(node);return()=>observer.disconnect();
  },[]);
  const [manual,setManual]=useState<'commands'|'skills'|null>(null);
  const [dismissed,setDismissed]=useState<string|null>(null);
  const [query,setQuery]=useState('');
  const [catalog,setCatalog]=useState<Skill[]>([]);
  const [loading,setLoading]=useState(false);
  const [failure,setFailure]=useState('');
  const [warnings,setWarnings]=useState('');
  const [reload,setReload]=useState(0);
  const token=composerToken(value);
  const mode=manual ?? (dismissed===value?null:token?.kind);
  const search=manual?query:token?.query??'';
  const binding=`${project?.runtime}:${project?.root}:${project?.generation}`;
  useEffect(()=>{setManual(null);setCatalog([]);setFailure('');setWarnings('');},[binding]);
  useEffect(()=>{
    if (mode!=='skills'||project?.status!=='ready') return;
    if(project.runtime==='opencode'){setFailure('OpenCode skill attachments are not exposed through ACP. Its configured skills remain runtime-managed.');setCatalog([]);return;}
    let disposed=false;setLoading(true);setFailure('');setWarnings('');
    invoke<{data:{cwd:string;skills:Skill[];errors:{message:string}[]}[]}>('conversation_request',{runtime:project.runtime,root:project.root,generation:project.generation,action:{type:'listSkills',forceReload:true}}).then(result=>{
      if(disposed)return;
      const entries=result.data.filter(entry=>entry.cwd===project.root);
      setCatalog(entries.flatMap(entry=>entry.skills));
      setWarnings(entries.flatMap(entry=>entry.errors??[]).map(error=>error.message).join(' · '));
    }).catch(error=>{if(!disposed){setCatalog([]);setFailure(errorText(error));}}).finally(()=>{if(!disposed)setLoading(false);});
    return()=>{disposed=true;};
  },[mode,binding,project?.status,reload,invoke,errorText]);
  const rows=mode==='skills'?catalog.filter(skill=>`${skill.name} ${skill.description} ${skill.interface?.displayName??''}`.toLowerCase().includes(search.toLowerCase())).map(skill=>({id:skill.path,name:`$${skill.name}`,label:skill.interface?.displayName||skill.name,description:skill.interface?.shortDescription||skill.shortDescription||skill.description,unavailable:!skill.enabled,skill})):composerCommands.filter(command=>`${command.id} ${command.label}`.toLowerCase().includes(search.toLowerCase())).map(command=>({...command,unavailable:Boolean(command.unavailable||(project?.runtime==='opencode'&&['review','compact'].includes(command.id))),name:`/${command.id}`,skill:undefined}));
  function close(){setManual(null);setDismissed(value);setQuery('');textarea.current?.focus();}
  function choose(row:typeof rows[number]) {
    if(row.unavailable)return;
    if(row.skill){
      const skill=row.skill;
      const text=token ? value.slice(0,token.start)+`$${skill.name} ` : `${value}${value&&!/\s$/.test(value)?' ':''}$${skill.name} `;
      onSkills([...skills.filter(old=>old.path!==skill.path),skill]);onChange(text);setManual(null);setQuery('');setDismissed(text);textarea.current?.focus();
    } else if(row.id==='skills'){setManual('skills');setQuery('');if(token)onChange('');}
    else {
      if(value.trim()&&!token){setFailure('Keep or send your draft before running a command.');return;}
      setManual(null);setDismissed('');onChange('');onCommand(row.id);textarea.current?.focus();
    }
  }
  return <div className="composer-input">
    {mode&&<section className="composer-picker" aria-label={mode==='skills'?'Available skills':'Composer commands'} onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();close();}if(event.key==='ArrowDown'&&event.target instanceof HTMLInputElement){event.preventDefault();event.currentTarget.querySelector<HTMLButtonElement>('.composer-options button:not(:disabled)')?.focus();}}}>
      <header><strong>{mode==='skills'?'Project skills':'Commands'}</strong><span>{mode==='skills'?'Reported by Codex':'CorpsTex actions'}</span><button type="button" aria-label="Close command picker" onClick={close}>×</button></header>
      {manual&&<input type="search" autoFocus aria-label={`Search ${mode}`} placeholder={`Search ${mode}…`} value={query} onChange={event=>setQuery(event.target.value)}/>}
      {loading&&<p role="status">Loading skills…</p>}{failure&&<p role="alert">{failure}</p>}{warnings&&<p role="status">Some skills could not load: {warnings}</p>}
      <div className="composer-options" onKeyDown={event=>{
        if(!['ArrowDown','ArrowUp','Home','End'].includes(event.key))return;
        const buttons=Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
        if(!buttons.length)return;event.preventDefault();const at=buttons.indexOf(document.activeElement as HTMLButtonElement);buttons[event.key==='Home'?0:event.key==='End'?buttons.length-1:(at+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length]?.focus();
      }}>{rows.map(row=><button type="button" key={row.id} disabled={row.unavailable||loading} onClick={()=>choose(row)}><code>{row.name}</code><span><strong>{row.label}{row.unavailable?' · unavailable':''}</strong><small>{row.description}</small></span></button>)}</div>
      {!rows.length&&!loading&&!failure&&<p>No matches.</p>}
      <footer><span>↓ to browse · Enter to choose · Esc to close</span>{mode==='skills'&&<button type="button" disabled={loading} onClick={()=>setReload(n=>n+1)}>Refresh skills</button>}</footer>
    </section>}
    {skills.length>0&&<div className="skill-attachments" aria-label="Attached skills">{skills.map(skill=><button type="button" key={skill.path} title={skill.path} aria-label={`Remove ${skill.name} skill attachment`} onClick={()=>onSkills(skills.filter(old=>old.path!==skill.path))}>${skill.name} <span aria-hidden="true">×</span></button>)}</div>}
    {imageError&&<p role="alert">{imageError}</p>}
    {images.length>0&&<div className="image-attachments" aria-label="Attached images">{images.map((image,index)=><div key={index}><img src={image.url} alt={image.name}/><span>{image.name}</span><button type="button" aria-label={`Remove image ${index+1}: ${image.name}`} onClick={()=>onImages(images.filter((_,at)=>at!==index))}>×</button></div>)}</div>}
    <textarea onPaste={event=>{const files=Array.from(event.clipboardData.items).filter(item=>item.kind==='file').map(item=>item.getAsFile()).filter((file):file is File=>Boolean(file));if(files.length){event.preventDefault();void attach(files);}}} ref={textarea} id={idPrefix?`${idPrefix}-message`:'message'} rows={2} value={value} onChange={event=>{setDismissed(null);setManual(null);onChange(event.target.value);}} onKeyDown={event=>{
      if(event.nativeEvent.isComposing)return;
      if(event.key==='Escape'&&mode){event.preventDefault();close();return;}
      if(event.key==='ArrowDown'&&mode){const first=event.currentTarget.parentElement?.querySelector<HTMLButtonElement>('.composer-options button:not(:disabled)');if(first){event.preventDefault();first.focus();}return;}
      if(event.key==='Enter'&&mode&&token&&!event.altKey&&!event.ctrlKey&&!event.metaKey){event.preventDefault();const exact=rows.find(row=>row.name===(mode==='skills'?'$':'/')+token.query);if(exact)choose(exact);else event.currentTarget.parentElement?.querySelector<HTMLButtonElement>('.composer-options button:not(:disabled)')?.focus();return;}
      if(event.key==='Enter'&&event.shiftKey&&!event.altKey&&!event.ctrlKey&&!event.metaKey){event.preventDefault();onSend();}
    }} disabled={disabled} aria-describedby={(selected?['composer-help','message-destination']:['composer-help']).map(id=>idPrefix?`${idPrefix}-${id}`:id).join(' ')} placeholder={selected?'Ask or describe a task…':'Choose a conversation'}/>
    <div className="composer-tools"><input ref={fileInput} type="file" accept="image/png,image/jpeg,image/gif,image/webp" multiple hidden onChange={event=>{const files=Array.from(event.target.files??[]);event.target.value='';void attach(files);}}/><button type="button" aria-label={reading?'Reading images…':'Attach images'} title="Attach images (or paste from clipboard)" disabled={disabled||reading} onClick={()=>fileInput.current?.click()}><svg aria-hidden="true" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6"><path d="m8 12 6-6a3 3 0 0 1 4 4l-8 8a5 5 0 0 1-7-7l9-9"/><path d="m6 14 8-8"/></svg></button><button type="button" aria-label="Commands" title="Commands (type /)" disabled={disabled} aria-expanded={mode==='commands'} onClick={()=>{setManual(mode==='commands'?null:'commands');setQuery('');}}> <span aria-hidden="true">/</span></button><button type="button" aria-label="Skills" title="Skills (type $)" disabled={disabled||project?.status!=='ready'} aria-expanded={mode==='skills'} onClick={()=>{setManual(mode==='skills'?null:'skills');setQuery('');}}> <span aria-hidden="true">$</span></button></div>{dockActions}<div className="composer-configuration">{trailing}</div>
  </div>;
}
