import { useId, useState } from 'react';

export type Question = { id: string; question: string; header?: string; isSecret?: boolean; isOther?: boolean; options?: { label: string; description?: string }[] };
const byteLength = (value: string) => new TextEncoder().encode(value).length;
export function completeAnswers(questions: Question[], answers: Record<string, string>) {
  return questions.length > 0 && questions.every(q => {
    const answer = answers[q.id];
    return typeof answer === 'string' && answer.trim().length > 0 && byteLength(answer) <= 16384 &&
      (!q.options?.length || q.isOther || q.options.some(option => option.label === answer));
  });
}

export function QuestionForm({ questions, blocked, onSubmit }: { questions: Question[]; blocked: boolean; onSubmit: (answers: Record<string, string>) => void }) {
  const prefix = useId();
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [custom, setCustom] = useState<Record<string, boolean>>({});
  const answered = questions.filter(q => completeAnswers([q], answers)).length;
  function update(id: string, value: string) { setAnswers(previous => ({ ...previous, [id]: value })); }
  return <form className="question-form" onSubmit={event => {
    event.preventDefault();
    if (!blocked && completeAnswers(questions, answers)) onSubmit(Object.fromEntries(questions.map(q => [q.id, answers[q.id]])));
  }}>
    <p className="question-progress" role="status">{answered} of {questions.length} answered · Review your choices, then send.</p>
    {questions.map((q, index) => {
      const name = `${prefix}-${index}`;
      const hasOptions = Boolean(q.options?.length);
      const writing = !hasOptions || custom[q.id];
      return <fieldset className="question-card" key={q.id} disabled={blocked}>
        <legend><span className="question-number">{index + 1}</span> {q.header || 'Your response'}</legend>
        <p className="question-prompt" id={`${name}-prompt`}>{q.question}</p>
        {hasOptions && <div className="question-options" role="radiogroup" aria-labelledby={`${name}-prompt`}>
          {q.options!.map((option, i) => <label className="answer-option" key={i}>
            <input type="radio" name={name} checked={!writing && answers[q.id] === option.label} value={option.label} onChange={() => { setCustom(previous => ({ ...previous, [q.id]: false })); update(q.id, option.label); }} />
            <span><strong>{option.label}</strong>{option.description && <small>{option.description}</small>}</span>
          </label>)}
          {q.isOther && <label className="answer-option"><input type="radio" name={name} checked={Boolean(custom[q.id])} onChange={() => { setCustom(previous => ({ ...previous, [q.id]: true })); update(q.id, ''); }} /><span><strong>Write my own answer</strong></span></label>}
        </div>}
        {writing && <label className="written-answer" htmlFor={`${name}-text`}>
          <span>{q.isSecret ? 'Private answer' : 'Your answer'}</span>
          {q.isSecret ? <input id={`${name}-text`} aria-describedby={`${name}-prompt`} type="password" autoComplete="off" required maxLength={16384} value={answers[q.id] ?? ''} onChange={e => update(q.id, e.target.value)} /> : <textarea id={`${name}-text`} aria-describedby={`${name}-prompt`} rows={3} required maxLength={16384} value={answers[q.id] ?? ''} onChange={e => update(q.id, e.target.value)} />}
        </label>}
      </fieldset>;
    })}
    <button className="send-answers" type="submit" disabled={blocked || !completeAnswers(questions, answers)}>{blocked ? 'Response unavailable' : 'Send answers'}</button>
  </form>;
}
