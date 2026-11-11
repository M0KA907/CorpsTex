import * as monaco from 'monaco-editor/editor/editor.api';
import 'monaco-editor/features/register.all';
import 'monaco-editor/languages/definitions/register.all';
import EditorWorker from 'monaco-editor/editor/editor.worker?worker';
import { fileUri, getEditor, bindEditorInvoke, language, message, notice, openFile, type Invoke, type FileTab, type ServerConfig } from './state';
import { LspNotificationQueue } from './lspQueue';

(globalThis as typeof globalThis & {MonacoEnvironment:unknown}).MonacoEnvironment={getWorker:()=>new EditorWorker()};
monaco.editor.defineTheme('corpstex',{base:'vs-dark',inherit:true,rules:[{token:'comment',foreground:'98948a'},{token:'keyword',foreground:'e4a68d'},{token:'string',foreground:'bbceaa'}],colors:{'editor.background':'#262624','editor.foreground':'#efede5','editorLineNumber.foreground':'#98948a','editorCursor.foreground':'#e4a68d','editor.selectionBackground':'#70534877','editor.lineHighlightBackground':'#2d2d2a'}});
export {monaco};
export const canonicalFileUri=(root:string,path='')=>monaco.Uri.parse(fileUri(root,path)).toString();
const canonicalUri=(uri:string)=>monaco.Uri.parse(uri).toString();
const models=new Map<string,monaco.editor.ITextModel>();
export function modelFor(file:FileTab):monaco.editor.ITextModel{
  let model=models.get(file.id);
  if(!model){model=monaco.editor.createModel(file.text,language(file.path),monaco.Uri.parse(canonicalFileUri(file.root,file.path)));models.set(file.id,model);for(const server of servers.values())attach(server,model);}
  return model;
}
export function disposeClosedModels(ids:Set<string>){for(const [id,model] of models){if(!ids.has(id)){model.dispose();models.delete(id);}}}
monaco.editor.registerEditorOpener({openCodeEditor:async(_source,resource,selection)=>{
  const root=getEditor().files.find(f=>resource.toString().startsWith(canonicalFileUri(f.root)+'/'))?.root;
  if(!root)return false;
  const path=decodeURIComponent(resource.toString().slice(canonicalFileUri(root).length+1));
  const cursor=selection?{lineNumber:'startLineNumber' in selection?selection.startLineNumber:selection.lineNumber,column:'startColumn' in selection?selection.startColumn:selection.column}:undefined;
  await openFile(root,path,true,cursor);return true;
}});
type Json=Record<string,any>;
type Running={invoke:Invoke;root:string;config:ServerConfig;cap:Json;queue:LspNotificationQueue;disposables:monaco.IDisposable[];models:Map<string,monaco.IDisposable[]>;timer:ReturnType<typeof setInterval>;polling:boolean;status:string};
export const servers=new Map<string,Running>();
const key=(root:string,id:string)=>JSON.stringify([root,id]);
const match=(s:Running,m:monaco.editor.ITextModel)=>m.uri.toString().startsWith(canonicalFileUri(s.root)+'/')&&s.config.language.split(',').map(l=>l.trim()).includes(m.getLanguageId());
const range=(r:Json):monaco.IRange=>({startLineNumber:r.start.line+1,startColumn:r.start.character+1,endLineNumber:r.end.line+1,endColumn:r.end.character+1});
const position=(p:monaco.Position)=>({line:p.lineNumber-1,character:p.column-1});
const completionKinds=[monaco.languages.CompletionItemKind.Text,monaco.languages.CompletionItemKind.Method,monaco.languages.CompletionItemKind.Function,monaco.languages.CompletionItemKind.Constructor,monaco.languages.CompletionItemKind.Field,monaco.languages.CompletionItemKind.Variable,monaco.languages.CompletionItemKind.Class,monaco.languages.CompletionItemKind.Interface,monaco.languages.CompletionItemKind.Module,monaco.languages.CompletionItemKind.Property,monaco.languages.CompletionItemKind.Unit,monaco.languages.CompletionItemKind.Value,monaco.languages.CompletionItemKind.Enum,monaco.languages.CompletionItemKind.Keyword,monaco.languages.CompletionItemKind.Snippet,monaco.languages.CompletionItemKind.Color,monaco.languages.CompletionItemKind.File,monaco.languages.CompletionItemKind.Reference,monaco.languages.CompletionItemKind.Folder,monaco.languages.CompletionItemKind.EnumMember,monaco.languages.CompletionItemKind.Constant,monaco.languages.CompletionItemKind.Struct,monaco.languages.CompletionItemKind.Event,monaco.languages.CompletionItemKind.Operator,monaco.languages.CompletionItemKind.TypeParameter];
const lspRange=(r:monaco.IRange)=>({start:{line:r.startLineNumber-1,character:r.startColumn-1},end:{line:r.endLineNumber-1,character:r.endColumn-1}});
const params=(m:monaco.editor.ITextModel,p:monaco.Position)=>({textDocument:{uri:m.uri.toString()},position:position(p)});
function markdown(contents:unknown):monaco.IMarkdownString[]{
  if(Array.isArray(contents))return contents.flatMap(markdown);
  if(typeof contents==='string')return [{value:contents,isTrusted:false,supportHtml:false}];
  if(contents&&typeof contents==='object'){
    const v=contents as Json;if(typeof v.value==='string')return [{value:v.kind==='plaintext'?v.value.replace(/[\\`*_{}[\]()#+.!<>]/g,'\\$&'):v.value,isTrusted:false,supportHtml:false}];
  }
  return [];
}
function send(s:Running,method:string,params:unknown,fullTextUri?:string){
  s.queue.enqueue(method,params,fullTextUri);
}
async function request(s:Running,method:string,params:unknown){await s.queue.flush();return s.invoke<any>('editor_lsp_request',{root:s.root,id:s.config.id,method,params});}
function attach(s:Running,model:monaco.editor.ITextModel){
  if(!match(s,model)||s.models.has(model.uri.toString())||model.getValueLength()>2*1024*1024||getEditor().files.some(f=>f.large&&canonicalFileUri(f.root,f.path)===model.uri.toString()))return;
  const uri=model.uri.toString();const sync=s.cap.textDocumentSync;const kind=typeof sync==='number'?sync:sync?.change;
  send(s,'textDocument/didOpen',{textDocument:{uri,languageId:model.getLanguageId(),version:model.getVersionId(),text:model.getValue()}});
  const change=model.onDidChangeContent(event=>{
    if(kind===0)return;
    send(s,'textDocument/didChange',{textDocument:{uri,version:model.getVersionId()},contentChanges:kind===2?event.changes.map(c=>({range:lspRange(c.range),rangeLength:c.rangeLength,text:c.text})):[{text:model.getValue()}]},kind===2?undefined:uri);
  });
  const dispose=model.onWillDispose(()=>{send(s,'textDocument/didClose',{textDocument:{uri}});s.models.delete(uri);change.dispose();});
  s.models.set(uri,[change,dispose]);
}
export async function startServer(root:string,config:ServerConfig){
  if(servers.has(key(root,config.id)))throw {message:'This language server is already running.'};
  const invoke=bindEditorInvoke();
  const init=await invoke<Json>('editor_lsp_start',{root,id:config.id,config:{executable:config.executable,args:config.args}});
  if(init.capabilities?.positionEncoding && init.capabilities.positionEncoding!=='utf-16'){
    await invoke('editor_lsp_stop',{root,id:config.id});throw {message:'Server selected unsupported position encoding; UTF-16 is required.'};
  }
  const queue=new LspNotificationQueue(
    (method,params)=>invoke('editor_lsp_notify',{root,id:config.id,method,params}),
    error=>{
      notice(`${config.name}: synchronization stopped. ${error.message} Your buffers are retained; restart the language server in Editor settings.`);
      queueMicrotask(()=>{if(servers.get(key(root,config.id))===s)void stopServer(root,config.id).catch(e=>notice(message(e)));});
    },
  );
  const s:Running={invoke,root,config,cap:init.capabilities??{},queue,disposables:[],models:new Map(),timer:0 as unknown as ReturnType<typeof setInterval>,polling:false,status:'Running'};
  servers.set(key(root,config.id),s);
  const selector=config.language.split(',').map(l=>l.trim());
  if(s.cap.completionProvider)s.disposables.push(monaco.languages.registerCompletionItemProvider(selector,{
    triggerCharacters:s.cap.completionProvider.triggerCharacters,
    provideCompletionItems:async(model,pos,context,token)=>{
      if(!match(s,model))return {suggestions:[]};
      const value=await request(s,'textDocument/completion',{...params(model,pos),context:{triggerKind:context.triggerKind+1,triggerCharacter:context.triggerCharacter}});
      if(token.isCancellationRequested)return {suggestions:[]};
      const word=model.getWordUntilPosition(pos);const fallback={startLineNumber:pos.lineNumber,endLineNumber:pos.lineNumber,startColumn:word.startColumn,endColumn:word.endColumn};
      const items=Array.isArray(value)?value:value?.items??[];
      return {incomplete:!Array.isArray(value)&&value?.isIncomplete,suggestions:items.slice(0,2000).map((i:Json)=>({
        label:i.label,kind:completionKinds[i.kind-1]??monaco.languages.CompletionItemKind.Text,detail:i.detail,documentation:markdown(i.documentation)[0],sortText:i.sortText,filterText:i.filterText,preselect:i.preselect,
        insertText:i.textEdit?.newText??i.insertText??i.label,insertTextRules:i.insertTextFormat===2?monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet:undefined,
        range:i.textEdit?.range?range(i.textEdit.range):i.textEdit?.insert?{insert:range(i.textEdit.insert),replace:range(i.textEdit.replace)}:fallback,
        additionalTextEdits:i.additionalTextEdits?.map((e:Json)=>({range:range(e.range),text:e.newText})),
      }))};
    },
  }));
  if(s.cap.hoverProvider)s.disposables.push(monaco.languages.registerHoverProvider(selector,{provideHover:async(model,pos,token)=>{
    if(!match(s,model))return null;const value=await request(s,'textDocument/hover',params(model,pos));return token.isCancellationRequested||!value?null:{contents:markdown(value.contents),range:value.range?range(value.range):undefined};
  }}));
  if(s.cap.definitionProvider)s.disposables.push(monaco.languages.registerDefinitionProvider(selector,{provideDefinition:async(model,pos,token)=>{
    if(!match(s,model))return null;const value=await request(s,'textDocument/definition',params(model,pos));if(token.isCancellationRequested||!value)return null;
    const entries=Array.isArray(value)?value:[value];if(entries.some((v:Json)=>!canonicalUri(String(v.uri??v.targetUri)).startsWith(canonicalFileUri(root)+'/')))notice('Some definitions are outside this project. Open their containing folder as a project to inspect them.');
    return entries.filter((v:Json)=>canonicalUri(String(v.uri??v.targetUri)).startsWith(canonicalFileUri(root)+'/')).map((v:Json)=>({uri:monaco.Uri.parse(v.uri??v.targetUri),range:range(v.range??v.targetSelectionRange)}));
  }}));
  for(const model of models.values())attach(s,model);
  s.timer=setInterval(()=>{if(s.polling)return;s.polling=true;void invoke<{alive:boolean;diagnostics:Json[];diagnosticsOmitted?:boolean}>('editor_lsp_poll',{root,id:config.id}).then(result=>{
    if(result.diagnosticsOmitted)notice(`${config.name}: some diagnostics exceeded the 4 MiB transport budget and were omitted.`);
    if(!result.alive){s.status='Stopped unexpectedly';notice(`${config.name}: ${s.status}`);void stopServer(root,config.id);return;}
    for(const event of result.diagnostics){const model=monaco.editor.getModel(monaco.Uri.parse(event.uri));if(!model||!match(s,model)||(typeof event.version==='number'&&event.version!==model.getVersionId()))continue;
      monaco.editor.setModelMarkers(model,config.id,(event.diagnostics??[]).slice(0,5000).filter((d:Json)=>d.range&&typeof d.message==='string').map((d:Json)=>({...range(d.range),message:d.message,source:d.source,code:typeof d.code==='string'||typeof d.code==='number'?String(d.code):undefined,severity:({1:monaco.MarkerSeverity.Error,2:monaco.MarkerSeverity.Warning,3:monaco.MarkerSeverity.Info,4:monaco.MarkerSeverity.Hint} as Record<number,monaco.MarkerSeverity>)[d.severity]??monaco.MarkerSeverity.Error})));
    }
  }).catch(e=>{s.status=message(e);notice(s.status);}).finally(()=>{s.polling=false;});},800);
  const saved=(event:Event)=>{const detail=(event as CustomEvent).detail;if(detail.root===root&&s.cap.textDocumentSync?.save){const uri=canonicalFileUri(root,detail.path);const model=monaco.editor.getModel(monaco.Uri.parse(uri));send(s,'textDocument/didSave',{textDocument:{uri},...(s.cap.textDocumentSync.save.includeText&&model?{text:model.getValue()}:{})});}};
  window.addEventListener('corpstex-editor-saved',saved);s.disposables.push({dispose:()=>window.removeEventListener('corpstex-editor-saved',saved)});
}
export async function stopServer(root:string,id:string){
  const s=servers.get(key(root,id));
  if(s){
    servers.delete(key(root,id));
    clearInterval(s.timer);for(const d of s.disposables)d.dispose();
    for(const [uri,ds] of s.models){send(s,'textDocument/didClose',{textDocument:{uri}});for(const d of ds)d.dispose();const m=monaco.editor.getModel(monaco.Uri.parse(uri));if(m)monaco.editor.setModelMarkers(m,id,[]);}
    await s.queue.flush().catch(()=>{});
  }
  if(s)await s.invoke('editor_lsp_stop',{root,id});
}
