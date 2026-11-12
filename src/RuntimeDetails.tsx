import { useEffect, useState } from 'react';
import './runtime-details.css';

export function runtimeDetailData(item:Record<string,unknown>):Record<string,unknown> {
  const data:Record<string,unknown>={};
  for(const key of ['command','cwd','exitCode','durationMs','changes','server','tool','arguments','error','success','senderThreadId','receiverThreadIds','agentsStates','agentThreadId','agentPath','kind','model','reasoningEffort','prompt','query','action','review']) {
    if(item[key]!=null)data[key]=item[key];
  }
  if(typeof item.clientProgress==='string')data.progress=item.clientProgress;
  if(item.result&&typeof item.result==='object'&&!Array.isArray(item.result)) {
    const result=item.result as {content?:unknown;structuredContent?:unknown;isError?:unknown};
    data.result={content:Array.isArray(result.content)?result.content.filter(entry=>entry&&typeof entry==='object'&&'type' in entry&&entry.type==='text'):[],structuredContent:result.structuredContent,isError:result.isError};
  }
  for(const key of ['output','contentItems']) {
    if(typeof item[key]==='string')data[key]=item[key];
    else if(Array.isArray(item[key]))data[key]=item[key].filter(entry=>entry&&typeof entry==='object'&&['text','inputText','input_text'].includes(entry.type));
  }
  return data;
}

export function RuntimeFields({item}:{item:Record<string,unknown>}) {
  const data=runtimeDetailData(item);
  const output=typeof item.aggregatedOutput==='string'?item.aggregatedOutput:typeof item.clientOutput==='string'?item.clientOutput:null;
  const omitted=typeof item.aggregatedOutput==='string'?0:typeof item.clientOutputOmittedBytes==='number'?item.clientOutputOmittedBytes:0;
  const text=JSON.stringify(data,null,2);
  const result=item.result as {content?:{type:string;text?:string}[]} | undefined;
  const toolText=Array.isArray(result?.content)?result.content.filter(entry=>entry&&typeof entry==='object'&&entry.type==='text'&&typeof entry.text==='string').map(entry=>entry.text).join('\n'):null;
  return <>
    {omitted>0&&<p role="status">Earlier live output omitted: {omitted} bytes. The completed runtime report may provide full output.</p>}
    {output!==null&&<pre className="message-body">{output}</pre>}
    {output===null&&toolText&&<pre className="message-body">{toolText}</pre>}
    {Object.keys(data).length>0&&<details className="runtime-raw"><summary>Reported fields</summary><pre className="message-body">{text.slice(0,131072)}</pre>
      {text.length>131072&&<p role="status">Details preview limited to 128 KiB of characters. Inspect the runtime report for the complete data.</p>}
    </details>}
    {item.reported!=null&&<details className="runtime-raw"><summary>Reported OpenCode data</summary><pre className="message-body">{JSON.stringify(item.reported,null,2)}</pre></details>}
  </>;
}

export function RuntimeItemDetails({item}:{item:Record<string,unknown>}) {
  const status=typeof item.clientActivityStatus==='string'?item.clientActivityStatus:typeof item.status==='string'?item.status:undefined;
  const result=item.result&&typeof item.result==='object'?item.result as {isError?:boolean}:null;
  const failed=status==='failed'||item.success===false||result?.isError===true||(typeof item.exitCode==='number'&&item.exitCode!==0);
  const [expanded,setExpanded]=useState(failed);
  useEffect(()=>{if(failed)setExpanded(true);},[failed]);
  const labels:Record<string,string>={commandExecution:'Command',fileChange:'File changes',mcpToolCall:'MCP tool',collabAgentToolCall:'Agent operation',webSearch:'Web search',dynamicToolCall:'Client tool',functionCallOutput:'Tool output',opencodeTool:'OpenCode activity',opencodeUpdate:'OpenCode activity',imageView:'View image',imageGeneration:'Generate image'};
  const kind=typeof item.type==='string'?item.type:'Runtime item';
  const tool=typeof item.tool==='string'&&typeof item.server==='string'?`${item.server}/${item.tool}`:item.tool;
  const change=Array.isArray(item.changes)&&typeof item.changes[0]?.path==='string'?`${item.changes[0].path}${item.changes.length>1?` +${item.changes.length-1}`:''}`:undefined;
  const target=[item.command,tool,item.query,item.path,change].find(value=>typeof value==='string'&&value.length>0);
  const statusLabels:Record<string,string>={inProgress:'Running',running:'Running',completed:'Completed',failed:'Failed',interrupted:'Stopped',declined:'Declined'};
  const error=typeof item.error==='string'?item.error:item.error&&typeof item.error==='object'&&'message' in item.error&&typeof item.error.message==='string'?item.error.message:null;
  return <details className="runtime-details" open={expanded} data-status={failed?'failed':status} onToggle={event=>setExpanded(event.currentTarget.open)}>
    <summary><span className="runtime-operation">{labels[kind]??kind}</span>{typeof target==='string'&&<code className="runtime-target" title={target}>{target}</code>}<span className="runtime-item-status">{failed?'Failed':status?statusLabels[status]??status:'Status not reported'}{typeof item.durationMs==='number'&&Number.isFinite(item.durationMs)&&item.durationMs>=0&&<small> · {item.durationMs} ms</small>}</span></summary>
    {failed&&error&&<p className="runtime-error" role="status">{error}</p>}
    {expanded&&<RuntimeFields item={item}/>}
  </details>;
}

export type TurnActivityData={clientPlan?:{explanation?:string|null;steps:{step:string;status:string}[]};clientDiff?:string};
export function TurnActivity({turn}:{turn:TurnActivityData}) {
  return <>
    {turn.clientPlan&&<section className="turn-plan" aria-label="Reported plan"><h3>Plan</h3>{turn.clientPlan.explanation&&<p>{turn.clientPlan.explanation}</p>}<ol>{turn.clientPlan.steps.map((step,index)=><li key={index}><span>{['pending','inProgress','completed'].includes(step.status)?step.status:`Unknown status: ${step.status}`}</span> {step.step}</li>)}</ol></section>}
    {turn.clientDiff&&<details className="runtime-details"><summary>Turn diff</summary><pre className="message-body">{turn.clientDiff}</pre></details>}
  </>;
}
