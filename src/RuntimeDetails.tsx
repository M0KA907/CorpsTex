import { useState } from 'react';
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
  return <>
    {omitted>0&&<p role="status">Earlier live output omitted: {omitted} bytes. The completed runtime report may provide full output.</p>}
    {output!==null&&<pre className="message-body">{output}</pre>}
    {Object.keys(data).length>0&&<pre className="message-body">{text.slice(0,131072)}</pre>}
    {text.length>131072&&<p role="status">Details preview limited to 128 KiB of characters. Inspect the runtime report for the complete data.</p>}
  </>;
}

export function RuntimeItemDetails({item}:{item:Record<string,unknown>}) {
  const [expanded,setExpanded]=useState(false);
  return <details className="runtime-details" onToggle={event=>setExpanded(event.currentTarget.open)}>
    <summary>Inspect {typeof item.type==='string'?item.type:'runtime item'}</summary>
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
