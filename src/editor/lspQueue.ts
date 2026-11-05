type Notification = {method:string;params:unknown;bytes:number;replaceKey?:string};
const MAX_COUNT=128;
const MAX_BYTES=8*1024*1024;

// Keep one in-flight notification and a finite backlog. Only adjacent full-text
// changes for the same document may replace each other; lifecycle messages and
// incremental edits must keep their exact order.
export class LspNotificationQueue {
  private pending:Notification[]=[];
  private bytes=0;
  private active=false;
  private running=false;
  private failure:Error|null=null;
  private waiters=new Set<{resolve:()=>void;reject:(error:Error)=>void}>();

  constructor(
    private deliver:(method:string,params:unknown)=>Promise<unknown>,
    private onFailure:(error:Error)=>void,
  ){}

  enqueue(method:string,params:unknown,fullTextUri?:string):void {
    if(this.failure)return;
    const bytes=new TextEncoder().encode(JSON.stringify({method,params})).byteLength;
    const replaceKey=method==='textDocument/didChange'?fullTextUri:undefined;
    const last=this.pending.at(-1);
    const replace=Boolean(replaceKey && last?.replaceKey===replaceKey);
    const count=this.pending.length+(this.active?1:0)+(replace?0:1);
    const total=this.bytes+bytes-(replace?last!.bytes:0);
    if(count>MAX_COUNT || total>MAX_BYTES){
      this.fail(new Error('Pending updates exceeded 128 messages or 8 MiB.'));
      return;
    }
    const entry={method,params,bytes,replaceKey};
    if(replace)this.pending[this.pending.length-1]=entry;
    else this.pending.push(entry);
    this.bytes=total;
    if(!this.running){
      this.running=true;
      queueMicrotask(()=>{void this.drain();});
    }
  }

  async flush():Promise<void>{
    if(this.failure)throw this.failure;
    if(!this.running)return;
    if(this.waiters.size>=MAX_COUNT)throw new Error('Too many requests waiting for document synchronization.');
    return new Promise<void>((resolve,reject)=>this.waiters.add({resolve,reject}));
  }

  private fail(error:Error){
    if(this.failure)return;
    this.failure=error;
    this.pending=[];
    this.bytes=0;
    for(const waiter of this.waiters)waiter.reject(error);
    this.waiters.clear();
    this.onFailure(error);
  }

  private async drain(){
    try{
      while(!this.failure && this.pending.length){
        const entry=this.pending.shift()!;
        this.active=true;
        let timer:ReturnType<typeof setTimeout>|undefined;
        try{
          await Promise.race([
            this.deliver(entry.method,entry.params),
            new Promise<never>((_resolve,reject)=>{timer=setTimeout(()=>reject(new Error('Notification delivery timed out.')),5000);}),
          ]);
        }finally{clearTimeout(timer);this.active=false;}
        if(!this.failure)this.bytes-=entry.bytes;
      }
    }catch(error){
      this.fail(error instanceof Error?error:new Error('Notification delivery failed.'));
    }finally{
      this.running=false;
      for(const waiter of this.waiters)waiter.resolve();
      this.waiters.clear();
    }
  }
}
