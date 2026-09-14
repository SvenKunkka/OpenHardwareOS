import type { ReactNode } from 'react';

/** Shared, presentational building blocks. No data fetching happens here. */

export type Tone = 'neutral' | 'ok' | 'warn' | 'danger' | 'info' | 'accent';

export function Badge({
  tone = 'neutral',
  children,
  title,
  plain = false,
}: {
  tone?: Tone;
  children: ReactNode;
  title?: string;
  /** Drop the leading dot when the badge is purely descriptive. */
  plain?: boolean;
}) {
  const classes = ['badge'];
  if (tone !== 'neutral') classes.push(`badge--${tone}`);
  if (plain) classes.push('badge--plain');
  return (
    <span className={classes.join(' ')} title={title}>
      {children}
    </span>
  );
}

export function Panel({
  title,
  subtitle,
  actions,
  children,
  className,
  id,
  flush = false,
}: {
  title?: ReactNode;
  subtitle?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  id?: string;
  flush?: boolean;
}) {
  return (
    <section className={['panel', className].filter(Boolean).join(' ')} id={id}>
      {title ? (
        <header className="panel__header">
          <div>
            <h2 className="panel__title">{title}</h2>
            {subtitle ? <p className="panel__subtitle">{subtitle}</p> : null}
          </div>
          {actions ? <div className="panel__actions">{actions}</div> : null}
        </header>
      ) : null}
      <div className={flush ? 'panel__body panel__body--flush' : 'panel__body'}>{children}</div>
    </section>
  );
}

export function SectionHead({
  title,
  hint,
  actions,
}: {
  title: ReactNode;
  hint?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="section__head">
      <h2 className="section__title">{title}</h2>
      {hint ? <p className="section__hint">{hint}</p> : null}
      {actions ? <div className="spacer">{actions}</div> : null}
    </div>
  );
}

export function Field({
  label,
  hint,
  htmlFor,
  children,
  error,
  labelAs = 'label',
}: {
  label: string;
  hint?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  error?: string;
  /**
   * Use `labelAs="span"` when the control inside is itself a `<label>` (a
   * checkbox or a switch): nested labels are invalid HTML, so the wrapper
   * becomes a labelled group instead.
   */
  labelAs?: 'label' | 'span';
}) {
  if (labelAs === 'span') {
    return (
      <div className="field" role="group" aria-label={label}>
        <span className="field__label">{label}</span>
        {children}
        {error ? <p className="field__error">{error}</p> : null}
        {hint ? <p className="field__hint">{hint}</p> : null}
      </div>
    );
  }
  return (
    <div className="field">
      <label className="field__label" htmlFor={htmlFor}>
        {label}
      </label>
      {children}
      {error ? <p className="field__error">{error}</p> : null}
      {hint ? <p className="field__hint">{hint}</p> : null}
    </div>
  );
}

export function Switch({
  id,
  checked,
  onChange,
  label,
  hint,
  disabled = false,
}: {
  id: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  hint?: ReactNode;
  disabled?: boolean;
}) {
  return (
    <label className="checkbox" htmlFor={id}>
      <span className="switch">
        <input
          id={id}
          type="checkbox"
          checked={checked}
          disabled={disabled}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span className="switch__track" aria-hidden="true">
          <span className="switch__thumb" />
        </span>
      </span>
      <span className="checkbox__text">
        <span className="checkbox__title">{label}</span>
        {/* A separating space: without it the label and the hint run together in
            the accessible name ("readingOff = …"). */}
        {hint ? <span className="checkbox__hint">{' '}{hint}</span> : null}
      </span>
    </label>
  );
}

export function Checkbox({
  id,
  checked,
  onChange,
  label,
  hint,
}: {
  id: string;
  checked: boolean;
  onChange: (next: boolean) => void;
  label: ReactNode;
  hint?: ReactNode;
}) {
  return (
    <label className="checkbox" htmlFor={id}>
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="checkbox__text">
        <span className="checkbox__title">{label}</span>
        {hint ? <span className="checkbox__hint">{' '}{hint}</span> : null}
      </span>
    </label>
  );
}

export function Segmented<T extends string>({
  legend,
  value,
  options,
  onChange,
}: {
  legend: string;
  value: T;
  options: { value: T; label: string; hint?: string }[];
  onChange: (next: T) => void;
}) {
  return (
    <fieldset className="field" style={{ border: 'none', margin: 0, padding: 0 }}>
      <legend className="field__label">{legend}</legend>
      <div className="segmented" role="group" aria-label={legend}>
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            className="segmented__option"
            aria-pressed={value === option.value}
            title={option.hint}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        ))}
      </div>
    </fieldset>
  );
}

export function KeyValue({ rows }: { rows: { key: string; value: ReactNode }[] }) {
  return (
    <dl className="kv">
      {rows.map((row) => (
        <div key={row.key} style={{ display: 'contents' }}>
          <dt className="kv__key">{row.key}</dt>
          <dd className="kv__value" style={{ margin: 0 }}>
            {row.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function InlineNotice({
  tone = 'neutral',
  title,
  children,
}: {
  tone?: 'neutral' | 'ok' | 'warn' | 'error';
  title?: ReactNode;
  children?: ReactNode;
}) {
  const modifier =
    tone === 'error' ? ' inline-notice--error' : tone === 'warn' ? ' inline-notice--warn' : tone === 'ok' ? ' inline-notice--ok' : '';
  return (
    <div className={`inline-notice${modifier}`} role={tone === 'error' ? 'alert' : undefined}>
      {title ? <p className="strong">{title}</p> : null}
      {children}
    </div>
  );
}

/**
 * Empty state that always explains how to get data. Every list uses this instead
 * of silently rendering nothing.
 */
export function EmptyState({
  title,
  body,
  steps,
  actions,
}: {
  title: string;
  body?: ReactNode;
  steps?: ReactNode[];
  actions?: ReactNode;
}) {
  return (
    <div className="empty">
      <h3 className="empty__title">{title}</h3>
      {body ? <p className="empty__body">{body}</p> : null}
      {steps && steps.length > 0 ? (
        <ol className="empty__steps">
          {steps.map((step, index) => (
            <li key={index}>
              <span className="empty__index" aria-hidden="true">
                {index + 1}.
              </span>
              <span>{step}</span>
            </li>
          ))}
        </ol>
      ) : null}
      {actions ? <div className="row">{actions}</div> : null}
    </div>
  );
}

export function Spinner({ label }: { label: string }) {
  return (
    <span className="row" role="status">
      <span className="spin" aria-hidden="true" />
      <span className="muted">{label}</span>
    </span>
  );
}
