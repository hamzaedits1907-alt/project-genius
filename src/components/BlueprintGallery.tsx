// "Start from a blueprint": curated, detailed starting points on Home.
import { useState } from 'react';
import { Rocket } from 'lucide-react';
import { BLUEPRINTS, BLUEPRINT_FILTERS, filterBlueprints, type Blueprint, type BlueprintFilter } from '../lib/blueprints';
import { BlueprintArt } from './BlueprintArt';

const CATEGORY_LABEL: Record<Blueprint['category'], string> = { website: 'Website', app: 'App', game: 'Game', tool: 'Tool' };

export function BlueprintGallery({
  onUse,
  onBuild,
  canBuild,
  building,
  selected,
}: {
  /** Put the blueprint's brief in the idea box to customise it. */
  onUse: (b: Blueprint) => void;
  /** Build straight away. */
  onBuild: (b: Blueprint) => void;
  canBuild: boolean;
  /** Id of the blueprint being built right now. */
  building?: string | null;
  /** Id of the blueprint currently loaded in the idea box. */
  selected?: string | null;
}) {
  const [filter, setFilter] = useState<BlueprintFilter>('all');
  const shown = filterBlueprints(BLUEPRINTS, filter);
  return (
    <section className="bp-section" aria-labelledby="bp-title">
      <div className="bp-head">
        <div>
          <h2 id="bp-title">Start from a blueprint</h2>
          <p className="muted small">Detailed, proven briefs. Pick one, make it yours, and Genius builds it.</p>
        </div>
        <div className="bp-filters" role="group" aria-label="Blueprint category">
          {BLUEPRINT_FILTERS.map((f) => (
            <button key={f.id} type="button" className={filter === f.id ? 'on' : ''} aria-pressed={filter === f.id} onClick={() => setFilter(f.id)}>
              {f.label}
              <span className="n">{filterBlueprints(BLUEPRINTS, f.id).length}</span>
            </button>
          ))}
        </div>
      </div>
      <div className="bp-grid" role="list">
        {shown.map((b) => (
          <article key={b.id} role="listitem" className={`bp-card ${selected === b.id ? 'selected' : ''}`} aria-label={b.title}>
            <button type="button" className="bp-main" onClick={() => onUse(b)} title="Put this brief in the idea box to customise it" aria-label={`Use blueprint: ${b.title}`}>
              <div className="bp-art-wrap">
                <BlueprintArt id={b.id} colors={b.colors} />
                <span className="bp-cat">{CATEGORY_LABEL[b.category]}</span>
              </div>
              <div className="bp-body">
                <h3>{b.title}</h3>
                <p>{b.description}</p>
                <div className="bp-tags">
                  {b.tags.map((t) => (
                    <span key={t}>{t}</span>
                  ))}
                </div>
              </div>
            </button>
            <div className="bp-foot">
              <span className="faint tiny">Click to customise</span>
              <button
                type="button"
                className="btn sm bp-build"
                disabled={!canBuild || !!building}
                onClick={() => onBuild(b)}
                aria-label={`Build ${b.title} now`}
                title={canBuild ? 'Create the project and start building right away' : 'Finish setup first (API key and model)'}
              >
                {building === b.id ? <span className="spinner" aria-hidden /> : <Rocket size={13} />} Build now
              </button>
            </div>
          </article>
        ))}
      </div>
    </section>
  );
}
