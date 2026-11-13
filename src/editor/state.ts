import { useSyncExternalStore } from 'react';
import { editorSessionKey, parseEditorSession, serializeEditorSession, type EditorSession, type EditorView } from './recovery';

export type Invoke = <T>(command:string,args?:Record<string,unknown>)=>Promise<T>;
const rawInvoke:Invoke = (command,args) => {
  const bridge=typeof window==='undefined'?undefined:(window as Window & {__TAURI__?:{core:{invoke:Invoke}}}).__TAURI__?.core.invoke;
  return bridge ? bridge(command,args) : Promise.reject({message:'File editing requires the CorpsTex desktop bridge.'});
};
const contextListeners=new Set<()=>void>();
export function onEditorContextChange(callback:()=>void){contextListeners.add(callback);return()=>{contextListeners.delete(callback);};}
function invalidateEditorContext(){for(const listener of contextListeners)listener();}
let contextKey='';
let contextOwner={project:'',runtime:'codex' as 'codex'|'opencode'};
let context:Promise<number>|null=null;
let contextQueue:Promise<unknown>=Promise.resolve();
export function setEditorContext(project:string,runtime:string):Promise<number>{
  const key=JSON.stringify([project,runtime]);
  if(context&&contextKey===key)return context;
  contextKey=key;
  contextOwner={project,runtime:runtime as 'codex'|'opencode'};
  const next=contextQueue.catch(()=>{}).then(()=>rawInvoke<number>('editor_context',{project,runtime}));
  context=next;contextQueue=next;
  invalidateEditorContext();
  void next.catch(()=>{if(context===next){context=null;contextKey='';}});
  return next;
}
export function bindEditorInvoke():Invoke {
  const selected=context;
  return async<T>(command:string,args?:Record<string,unknown>)=>{
    if(!selected||context!==selected)throw {kind:'grant',message:'Editor context changed. Retry in the current project. Your buffer is retained.'};
    const scope=await selected;
    if(context!==selected)throw {kind:'grant',message:'Editor context changed. Retry in the current project.'};
    const result=await rawInvoke<T>(command,{...args,scope});
    if(context!==selected)throw {kind:'grant',message:'Editor context changed. Stale result discarded; your buffer is retained.'};
    return result;
  };
}
export const invoke:Invoke = async(command,args) => {
  if(command==='editor_request'||command==='editor_grant'||['editor_lsp_start','editor_lsp_request','editor_lsp_notify','editor_lsp_poll','editor_lsp_stop'].includes(command)){
    return bindEditorInvoke()(command,args);
  }
  return rawInvoke(command,args);
};
export function message(error:unknown):string {
  const text=typeof error==='object'&&error!==null&&'message' in error?String(error.message):'Editor operation failed.';
  if(text.startsWith('Folder access is not granted in this editor context.'))return 'Editor access required.';
  return text.replace(/ (?:Your (?:edited )?buffer (?:is|has been) retained\.|your buffer is retained\.)/gi,'');
}
export type Disk = {text:string;revision:string;encoding:string;bom:boolean;bytes:number};
export type FileTab = {id:string;root:string;path:string;project:string;runtime:'codex'|'opencode';unloaded?:boolean;recoveryUnsaved?:boolean;view?:EditorView;disk:Disk;text:string;preview:boolean;large:boolean;conflict?:Disk;missing?:string;saving?:boolean;saveError?:string;managing?:boolean;cursor?:{lineNumber:number;column:number}};
export type Options = {theme:string;fontFamily:string;fontSize:number;ligatures:boolean;tabSize:number;insertSpaces:boolean;minimap:boolean;wrap:boolean;preview:boolean};
export const defaults:Options={theme:'corpstex',fontFamily:'"Cascadia Code", "JetBrains Mono", monospace',fontSize:14,ligatures:false,tabSize:2,insertSpaces:true,minimap:false,wrap:false,preview:false};
export type ServerConfig={id:string;name:string;language:string;executable:string;args:string[]};
export const presets:ServerConfig[]=[
  {id:'typescript',name:'TypeScript / JavaScript',language:'typescript,javascript,typescriptreact,javascriptreact',executable:'typescript-language-server',args:['--stdio']},
  {id:'python',name:'Python · Pyright',language:'python',executable:'pyright-langserver',args:['--stdio']},
  {id:'rust',name:'Rust',language:'rust',executable:'rust-analyzer',args:[]},
  {id:'go',name:'Go',language:'go',executable:'gopls',args:[]},
  {id:'cpp',name:'C / C++',language:'c,cpp',executable:'clangd',args:[]},
  {id:'json',name:'JSON',language:'json',executable:'vscode-json-language-server',args:['--stdio']},
  {id:'yaml',name:'YAML',language:'yaml',executable:'yaml-language-server',args:['--stdio']},
];
export type Settings={global:Options;profiles:Record<string,Partial<Options>>;activeProfile:string;projects:Record<string,Partial<Options>>;servers:ServerConfig[];themes:Record<string,unknown>;languageMappings:Record<string,string>};
export function sanitizeOptions(value:unknown):Partial<Options>{
  if(!value || typeof value!=='object')return {};
  const v=value as Record<string,unknown>;const result:Partial<Options>={};
  for(const key of ['ligatures','insertSpaces','minimap','wrap','preview'] as const)if(typeof v[key]==='boolean')result[key]=v[key];
  if(typeof v.fontSize==='number' && v.fontSize>=8 && v.fontSize<=40)result.fontSize=v.fontSize;
  if(typeof v.tabSize==='number' && Number.isInteger(v.tabSize) && v.tabSize>=1 && v.tabSize<=8)result.tabSize=v.tabSize;
  if(typeof v.fontFamily==='string' && v.fontFamily.length<300)result.fontFamily=v.fontFamily;
  if(typeof v.theme==='string' && /^[\w-]{1,80}$/.test(v.theme))result.theme=v.theme;
  return result;
}
const freshSettings=():Settings=>({global:{...defaults},profiles:{},activeProfile:'',projects:{},servers:presets.map(p=>({...p})),themes:{},languageMappings:{}});
function loadSettings():Settings {
  try {
    if(typeof window==='undefined')return freshSettings();
    const raw=localStorage.getItem('corpstex.editor.settings.v1');if(!raw || raw.length>1024*1024)return freshSettings();
    const v=JSON.parse(raw);const s=freshSettings();s.global={...defaults,...sanitizeOptions(v.global)};
    for(const k of ['profiles','projects'] as const)if(v[k]&&typeof v[k]==='object')for(const [name,value] of Object.entries(v[k]).slice(0,200))s[k][name]=sanitizeOptions(value);
    if(typeof v.activeProfile==='string' && s.profiles[v.activeProfile])s.activeProfile=v.activeProfile;
    if(Array.isArray(v.servers))s.servers=v.servers.slice(0,32).filter((c:ServerConfig)=>c&&/^[\w-]{1,80}$/.test(c.id)&&typeof c.name==='string'&&typeof c.language==='string'&&typeof c.executable==='string'&&Array.isArray(c.args)&&c.args.every(a=>typeof a==='string'&&a.length<4096));
    s.languageMappings=parseLanguageMappings(v.languageMappings)??{};
    if(v.themes&&typeof v.themes==='object')s.themes=v.themes;
    return s;
  }catch{return freshSettings();}
}
export function resolveOptions(settings:Settings,root:string):Options{return {...defaults,...sanitizeOptions(settings.global),...sanitizeOptions(settings.profiles[settings.activeProfile]),...sanitizeOptions(settings.projects[root])};}
type Snapshot={files:FileTab[];active:string|null;settings:Settings;notice:string;recovery:EditorSession|null;sidebar:EditorSession['sidebar']};
function loadRecovery(){try{return typeof window==='undefined'?null:parseEditorSession(localStorage.getItem(editorSessionKey));}catch{return null;}}
let snapshot:Snapshot={files:[],active:null,settings:loadSettings(),notice:'',recovery:loadRecovery(),sidebar:'conversations'};
const listeners=new Set<()=>void>();
const serverSnapshot=snapshot;
let reportedDirty:boolean|undefined;
let persistTimer:ReturnType<typeof setTimeout>|undefined;
export function persistEditorSession(){
  if(snapshot.recovery||typeof window==='undefined')return;
  try{localStorage.setItem(editorSessionKey,serializeEditorSession(snapshot.files,snapshot.active,snapshot.sidebar));}
  catch{if(snapshot.notice!=='Editor session metadata could not be stored.')emit({notice:'Editor session metadata could not be stored.'});}
}
export function setEditorSidebar(sidebar:EditorSession['sidebar']){emit({sidebar});}
export function discardEditorRecovery(){emit({recovery:null});persistEditorSession();}
export function restoreEditorSession(session:EditorSession):boolean{
  if(snapshot.files.length||managing||opening.size){notice('Wait for pending opens and close current editor tabs before restoring the previous session. Your buffers are retained.');return false;}
  const checked=parseEditorSession(JSON.stringify(session));if(!checked){notice('The saved editor session is invalid.');return false;}
  const unsaved=checked.tabs.filter(t=>t.unsaved);
  if(!window.confirm(`Restore ${checked.tabs.length} editor tab references?\n\nOnly paths, layout and cursor/view metadata were stored. ${unsaved.length} tab(s) had unsaved edits; their text is UNRECOVERABLE. Loading uses the current saved file on disk.\n\nFolder access is not restored. Grant access explicitly for each project/runtime and extra folder before loading.`))return false;
  const files:FileTab[]=checked.tabs.map(t=>({id:crypto.randomUUID(),root:t.root,path:t.path,project:t.project,runtime:t.runtime,preview:t.preview,cursor:t.cursor,view:t.view,recoveryUnsaved:t.unsaved,unloaded:true,large:false,text:'',disk:{text:'',revision:'',encoding:'utf8',bom:false,bytes:0}}));
  emit({files,active:checked.active===null?null:files[checked.active].id,sidebar:checked.sidebar,recovery:null});persistEditorSession();return true;
}
function emit(next:Partial<Snapshot>){
  snapshot={...snapshot,...next};
  if(next.files){const pending=snapshot.files.some(f=>dirty(f)||Boolean(f.conflict));if(pending!==reportedDirty){reportedDirty=pending;void invoke('editor_dirty',{dirty:pending}).catch(()=>{reportedDirty=undefined;});}}
  for(const l of listeners)l();
  if((next.files||'active' in next||'sidebar' in next||'recovery' in next)&&!snapshot.recovery&&typeof window!=='undefined'){clearTimeout(persistTimer);persistTimer=setTimeout(persistEditorSession,350);}
}
function subscribe(callback:()=>void){listeners.add(callback);return()=>{listeners.delete(callback);};}
export function useEditor(){return useSyncExternalStore(subscribe,()=>snapshot,()=>serverSnapshot);}
// Navigation observes only tab metadata. Text and view updates belong to the
// individual editor surface; they must not redraw retained conversations.
type NavigationFile=Pick<FileTab,'id'|'root'|'path'|'project'|'runtime'|'preview'|'saving'|'managing'> & {unsaved:boolean;conflicted:boolean};
let navigationSource:Snapshot|undefined;
let navigation:{files:NavigationFile[];active:string|null;sidebar:Snapshot['sidebar']}|undefined;
function editorNavigation(){
  if(navigationSource===snapshot)return navigation!;
  navigationSource=snapshot;
  const files=snapshot.files.map(f=>({id:f.id,root:f.root,path:f.path,project:f.project,runtime:f.runtime,preview:f.preview,saving:f.saving,managing:f.managing,unsaved:dirty(f),conflicted:Boolean(f.conflict)}));
  if(!navigation||navigation.active!==snapshot.active||navigation.sidebar!==snapshot.sidebar||JSON.stringify(navigation.files)!==JSON.stringify(files))navigation={files,active:snapshot.active,sidebar:snapshot.sidebar};
  return navigation;
}
export function useEditorNavigation(){return useSyncExternalStore(subscribe,editorNavigation,editorNavigation);}
export function useEditorFile(id:string){return useSyncExternalStore(subscribe,()=>snapshot.files.find(f=>f.id===id),()=>serverSnapshot.files.find(f=>f.id===id));}
export function useEditorSettings(){return useSyncExternalStore(subscribe,()=>snapshot.settings,()=>serverSnapshot.settings);}
export const getEditor=()=>snapshot;
export const dirty=(f:FileTab)=>f.text!==f.disk.text;
export const fs=<T,>(root:string,action:Record<string,unknown>)=>invoke<T>('editor_request',{root,action});
// Lookup only. A renderer path or cached promise cannot create a native grant.
export async function authorize(root:string):Promise<string>{return (await fs<{root:string}>(root,{type:'authorize'})).root;}
export const grantEditorAccess=(root:string)=>invoke<string>('editor_grant',{root});
export async function revokeEditorAccess(){
  const selected=context;
  if(!selected)return;
  const next=selected.then(scope=>rawInvoke<number>('editor_revoke',{scope}));
  context=next;contextQueue=next;
  invalidateEditorContext();
  try{await next;}catch(e){if(context===next){context=null;contextKey='';}throw e;}
}
const syncing=new Map<string,symbol>();
export function updateFile(id:string,update:Partial<FileTab>){
  if('disk' in update || 'conflict' in update || 'path' in update || update.saving===true || update.managing===true)syncing.delete(id);
  emit({files:snapshot.files.map(f=>f.id===id?{...f,...update,...((typeof update.text==='string'&&update.text.length>2*1024*1024)||(update.disk?.bytes??0)>2*1024*1024?{large:true}:{})}:f)});
}
export function setActiveFile(id:string|null){emit({active:id});}
export function notice(text:string){emit({notice:text});}
export function saveSettings(settings:Settings){const changed=JSON.stringify(settings.languageMappings)!==JSON.stringify(snapshot.settings.languageMappings);emit({settings});if(changed&&typeof window!=='undefined')window.dispatchEvent?.(new CustomEvent('corpstex-editor-language-mappings'));try{localStorage.setItem('corpstex.editor.settings.v1',JSON.stringify(settings));}catch{notice('Settings apply now, but could not be stored on this device.');}}
// Conservative UTF-16 text estimate plus any retained comparison snapshot.
const documentBytes=(disk:Disk)=>Math.max(disk.bytes,disk.text.length*2);
const bufferBytes=(file:FileTab)=>Math.max(documentBytes(file.disk),file.text.length*2)+(file.conflict?documentBytes(file.conflict):0);
const opening=new Map<string,Promise<void>>();
let managing=false;
let mutationVersion=0;
export async function openFile(root:string,path:string,permanent=false,cursor?:FileTab['cursor']):Promise<void>{
  const version=mutationVersion;const owner={...contextOwner};
  root=await authorize(root);
  if(managing||version!==mutationVersion){notice('A file operation changed this folder. Retry opening the file when it finishes.');return;}
  const existing=snapshot.files.find(f=>fileUri(f.root,f.path)===fileUri(root,path));
  if(existing){updateFile(existing.id,{preview:permanent?false:existing.preview,...(cursor?{cursor}:{})});setActiveFile(existing.id);return;}
  const key=fileUri(root,path);if(opening.has(key))return opening.get(key);
  if(snapshot.files.length+opening.size>=32){notice('Close a file tab before opening another (32 maximum, including pending opens).');return;}
  const task=(async()=>{
    try{
      let disk:Disk;let large=false;
      try{disk=await fs<Disk>(root,{type:'read',path,large:false});}catch(e){if((e as {kind?:string})?.kind!=='largeFile')throw e;if(!window.confirm('Open this large file? Files up to 32 MiB use reduced features; LSP and minimap will be disabled.'))return;large=true;disk=await fs<Disk>(root,{type:'read',path,large:true});}
      if(managing||version!==mutationVersion)throw {message:'A file operation changed this folder. Retry opening the file.'};
      if(snapshot.files.reduce((sum,f)=>sum+bufferBytes(f),0)+documentBytes(disk)>64*1024*1024)throw {message:'Open files exceed the 64 MiB estimated buffer budget. Close files before opening another.'};
      const preview=!permanent&&resolveOptions(snapshot.settings,root).preview;
      const files=snapshot.files.filter(f=>!(preview&&f.preview&&!dirty(f)&&!f.conflict&&!f.saving&&!f.managing));
      if(files.length>=32)throw {message:'Close a file tab before opening another (32 maximum).'};
      const file:FileTab={id:crypto.randomUUID(),root,path,...owner,disk,text:disk.text,preview,large,cursor};
      emit({files:[...files,file],active:file.id,notice:''});
    }catch(e){notice(message(e));}finally{opening.delete(key);}
  })();opening.set(key,task);return task;
}
export function closeFile(id:string):boolean{
  const f=snapshot.files.find(f=>f.id===id);if(!f || f.saving||f.managing)return false;
  if((dirty(f)||f.conflict)&&!window.confirm(`Discard unsaved changes to ${f.path}? There is no crash recovery storage.`))return false;
  syncing.delete(id);
  const files=snapshot.files.filter(f=>f.id!==id);emit({files,active:snapshot.active===id?files.at(-1)?.id??null:snapshot.active});return true;
}
export function discardFile(id:string){syncing.delete(id);const files=snapshot.files.filter(f=>f.id!==id);emit({files,active:snapshot.active===id?files.at(-1)?.id??null:snapshot.active});}
export async function saveFile(id:string){
  const file=snapshot.files.find(f=>f.id===id);if(!file || file.saving||file.managing||file.unloaded)return;
  if(file.conflict||file.missing){notice('Resolve the disk conflict before saving.');return;}
  if(!dirty(file))return;
  updateFile(id,{saving:true});
  try{
    const disk=await fs<Disk>(file.root,{type:'save',path:file.path,text:file.text,expected:file.disk.revision,encoding:file.disk.encoding,bom:file.disk.bom});
    updateFile(id,{disk,saving:false,preview:false,saveError:undefined});
    window.dispatchEvent(new CustomEvent('corpstex-editor-saved',{detail:{root:file.root,path:file.path}}));
  }catch(e){updateFile(id,{saving:false,saveError:message(e)});notice(message(e));if((e as {kind?:string})?.kind==='conflict')await syncFile(id);}
}
export async function syncFile(id:string){
  const file=snapshot.files.find(f=>f.id===id);if(!file || file.saving||file.managing||file.unloaded)return;
  const token=Symbol();syncing.set(id,token);
  try{
    const disk=await fs<Disk|null>(file.root,{type:'poll',path:file.path,expected:file.conflict?.revision??file.disk.revision});
    const current=snapshot.files.find(f=>f.id===id);if(!current||current.saving||syncing.get(id)!==token)return;
    if(disk){if(dirty(current))updateFile(id,{conflict:disk,missing:undefined});else updateFile(id,{disk,text:disk.text,conflict:undefined,missing:undefined});}
    else if(current.missing)updateFile(id,{missing:undefined});
  }catch(e){const current=snapshot.files.find(f=>f.id===id);const missing=message(e);if(current&&syncing.get(id)===token&&current.missing!==missing)updateFile(id,{missing});}
  finally{if(syncing.get(id)===token)syncing.delete(id);}
}
export function acceptDisk(id:string){const f=snapshot.files.find(f=>f.id===id);if(f?.conflict && window.confirm('Discard your buffer and load the current disk version?'))updateFile(id,{disk:f.conflict,text:f.conflict.text,conflict:undefined});}
export function acceptMerge(id:string){const f=snapshot.files.find(f=>f.id===id);if(f?.conflict && window.confirm('Use your edited buffer as the merge result? Ctrl+S will replace the disk version shown in the diff.'))updateFile(id,{disk:f.conflict,conflict:undefined,preview:false});}
export function parseLanguageMappings(value:unknown):Record<string,string>|null{
  if(!value||typeof value!=='object'||Array.isArray(value))return null;
  const entries=Object.entries(value);if(entries.length>200)return null;
  if(entries.some(([extension,id])=>!(/^[a-z0-9_-]{1,40}$/.test(extension))||['__proto__','constructor','prototype'].includes(extension)||typeof id!=='string'||!(/^[a-zA-Z][a-zA-Z0-9_.+-]{0,79}$/.test(id))))return null;
  return Object.fromEntries(entries) as Record<string,string>;
}
export function language(path:string):string{const ext=path.split('.').at(-1)?.toLowerCase()??'';const mapped=snapshot.settings.languageMappings?.[ext];if(typeof mapped==='string')return mapped;return ({ts:'typescript',tsx:'typescript',js:'javascript',jsx:'javascript',mjs:'javascript',json:'json',rs:'rust',py:'python',go:'go',c:'c',h:'cpp',cpp:'cpp',hpp:'cpp',css:'css',html:'html',md:'markdown',yaml:'yaml',yml:'yaml',toml:'ini',sh:'shell',sql:'sql',xml:'xml',java:'java',kt:'kotlin',cs:'csharp',txt:'plaintext'} as Record<string,string>)[ext]??'plaintext';}
export function fileUri(root:string,path=''):string {
  let normalized=root.replace(/\\/g,'/');
  if(normalized.startsWith('//?/UNC/'))normalized='//'+normalized.slice(8);
  else if(normalized.startsWith('//?/'))normalized=normalized.slice(4);
  const full=normalized.replace(/\/$/,'')+(path?'/'+path:'');
  const encoded=full.split('/').map(p=>encodeURIComponent(p).replace(/%3A/gi,':')).join('/');
  return full.startsWith('//')?'file:'+encoded:'file://'+(full.startsWith('/')?'':'/')+encoded;
}
export function relocatedPath(file:{root:string;path:string},root:string,source:string,destination:string,directory:boolean):string|null{
  const from=fileUri(root,source);const current=fileUri(file.root,file.path);
  if(current!==from&&(!directory||!current.startsWith(from+'/')))return null;
  return destination+decodeURIComponent(current.slice(from.length));
}

// Hold affected tabs through the complete native operation. Never close/re-read a
// moved buffer: consent or a context change may make a subsequent read impossible.
export async function manageFiles(root:string,source:string,destination:string,directory:boolean,type:'rename'|'delete'):Promise<boolean>{
  if(managing)throw {message:'Wait for the current file operation to finish.'};
  const affected=()=>snapshot.files.filter(f=>relocatedPath(f,root,source,destination,directory)!==null);
  if(affected().some(f=>dirty(f)||f.conflict||f.saving||f.managing||f.unloaded))throw {message:'Save or close the unsaved file before moving or deleting it.'};
  const request=bindEditorInvoke();
  const run=<T,>(action:Record<string,unknown>)=>request<T>('editor_request',{root,action});
  managing=true;mutationVersion++;
  for(const file of affected())updateFile(file.id,{managing:true});
  try{
    await run({type:'authorize'});
    let expected:string|undefined;
    if(type==='delete'){
      const inspected=await run<{revision:string|null;bytes:number;directory:boolean}>({type:'inspect',path:source});
      if(!window.confirm(`Permanently delete ${source}${inspected.directory?' (must be empty)':` (${inspected.bytes} bytes)`}? This cannot be undone.`))return false;
      expected=inspected.revision??undefined;
    }
    // Recheck after awaits and confirmation, before sending the mutation.
    if(affected().some(f=>dirty(f)||f.conflict||f.saving))throw {message:'The buffer changed. Save or close it before retrying the file operation.'};
    await run({type,path:source,...(type==='rename'?{destination}:{expected})});
    for(const file of affected()){
      if(type==='rename')updateFile(file.id,{root,path:relocatedPath(file,root,source,destination,directory)!,managing:false,preview:false});
      else if(dirty(file)||file.conflict||file.saving)updateFile(file.id,{managing:false,missing:'File deleted on disk. Your edited buffer is retained.'});
      else {updateFile(file.id,{managing:false});closeFile(file.id);}
    }
    return true;
  }finally{
    for(const file of snapshot.files)if(file.managing)updateFile(file.id,{managing:false});
    managing=false;mutationVersion++;
  }
}

export async function loadRecoveredFile(id:string){
  const file=snapshot.files.find(f=>f.id===id);if(!file?.unloaded||file.saving||managing)return;
  if(file.project!==contextOwner.project||file.runtime!==contextOwner.runtime){notice('Select the recovered tab’s project and runtime, then grant access before loading.');return;}
  const request=bindEditorInvoke();
  updateFile(id,{saving:true});
  try{
    await request('editor_request',{root:file.root,action:{type:'authorize'}});
    let disk:Disk;let large=false;
    try{disk=await request<Disk>('editor_request',{root:file.root,action:{type:'read',path:file.path,large:false}});}
    catch(e){if((e as {kind?:string})?.kind!=='largeFile')throw e;if(!window.confirm('Load this large file with reduced features (maximum 32 MiB)?'))return;large=true;disk=await request<Disk>('editor_request',{root:file.root,action:{type:'read',path:file.path,large:true}});}
    if(snapshot.files.reduce((sum,f)=>sum+(f.id===id?0:bufferBytes(f)),0)+documentBytes(disk)>64*1024*1024)throw {message:'Open files exceed the 64 MiB estimated buffer budget. Close files before loading another.'};
    const duplicate=snapshot.files.find(f=>f.id!==id&&!f.unloaded&&fileUri(f.root,f.path)===fileUri(file.root,file.path));
    if(duplicate){notice('This file is already open. Its current buffer is retained.');setActiveFile(duplicate.id);return;}
    updateFile(id,{disk,text:disk.text,large,unloaded:false,missing:undefined});notice('Loaded the current saved file. Previous unsaved text was not stored.');
  }catch(e){updateFile(id,{missing:message(e)});notice(message(e));}
  finally{updateFile(id,{saving:false});}
}
