import Link from '@docusaurus/Link';
import CodeBlock from '@theme/CodeBlock';
import Layout from '@theme/Layout';
import type { ReactNode } from 'react';
import HomepageHeader from '../components/Header';
import styles from './index.module.css';

const bindingExample = `#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => louder,
    Key::DownArrow.down() => quieter,
)]
pub struct VolumeLayer {
    volume: u8,
}`;

const handlerExample = `fn louder<'a>(_ev: &KeyEvent, node: Node<VolumeLayerPath<'a>, ()>) -> Vec<MercuryEffect> {
    let layer: &mut VolumeLayer = node.parent.get_mut();
    layer.volume = layer.volume + 10;
    vec![MercuryEffect::SetVolume(layer.volume)]
}`;

function Features() {
  return (
    <section>
      <div className="container">
        <h2 className={styles.centeredHeading}>Events in, effects out.</h2>
        <p>
          A <code>freddie</code> program ingests a stream of events and produces
          a stream of effects. One such event is generated when you press a key
          on your keyboard, and one such effect is a simulated keypress, so{' '}
          <code>freddie</code> can be used to build a key remapper. But the
          events and effects are arbitrary, and so <code>freddie</code> can be
          used to build something much more powerful.
        </p>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>A program, not a config file</h3>
            <p>
              You fork the repository, make the changes you want, and run{' '}
              <code>cargo build</code>. You respond to whatever events you want,
              you manage state however you choose, and your handlers receive
              that state.
            </p>
          </div>
          <div className="col col--4">
            <h3>One place the decision is made</h3>
            <p>
              This key was pressed, this app was foregrounded, this browser tab
              became active, this device connected. Emit this key, foreground
              this app, resize this window, run this arbitrary code. All of it
              flows through one model.
            </p>
          </div>
          <div className="col col--4">
            <h3>Pure, and therefore testable</h3>
            <p>
              <code>state.handle(event)</code> is a state transformer: state and
              event in, updated state and effects out. Effects are returned, not
              performed, so the whole keymap is a table you can assert on.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function BindingSection() {
  return (
    <section className="alt-background">
      <div className="container">
        <h2 className={styles.centeredHeading}>
          A binding is a trigger and a handler.
        </h2>
        <p>
          Say we want a volume layer, where <code>up</code> and <code>down</code>{' '}
          change the volume and the layer remembers what it set it to. The
          volume lives on the layer, because that is the only place it is used:
        </p>
        <div className={styles.codeBlockWrap}>
          <CodeBlock language="rust">{bindingExample}</CodeBlock>
        </div>
        <p>And the handler:</p>
        <div className={styles.codeBlockWrap}>
          <CodeBlock language="rust">{handlerExample}</CodeBlock>
        </div>
        <p>
          <code>node.parent</code> is the path to the level the binding was
          written on, so <code>get_mut</code> hands back this layer,
          unconditionally. There is no question of whether the volume layer is
          the active one. <code>louder</code> runs because it was, and the path
          is what says so. A state a binding cannot be reached in is not an arm
          that panics, it is a value the handler is never handed.
        </p>
      </div>
    </section>
  );
}

function Mercury() {
  return (
    <section>
      <div className="container">
        <h2 className={styles.centeredHeading}>Meet mercury.</h2>
        <p>
          This repository ships one program built with <code>freddie</code>,
          called <code>mercury</code>. It is macOS-only and it requires
          accessibility permissions. You should not expect it to fit your use
          case: it is here to be read, run, studied, forked, and modified.
        </p>
        <div className={styles.codeBlockWrap}>
          <CodeBlock language="bash">
            {`git clone https://github.com/freddiehg/freddie
cd freddie
cargo install --path crates/mercury
mercury`}
          </CodeBlock>
        </div>
        <p>
          <code>mercury</code> boots into the typing layer, where every
          keystroke passes through. Typing <code>jk</code> takes you home. From
          there, <code>n</code> is nav, <code>i</code> is in-app, <code>s</code>{' '}
          is per-site, <code>r</code> is resize, and <code>o</code> shows you an
          overlay of what is bound.
        </p>
        <div className={styles.ctaContainer}>
          <Link
            className="button button--primary button--lg"
            to="/docs/getting-started-with-mercury"
          >
            Get started
          </Link>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  return (
    <Layout
      title="freddie - a bespoke control plane for your computer"
      description="freddie is a set of tools for building a bespoke control plane for your computer. A freddie program ingests a stream of events and produces a stream of effects."
    >
      <HomepageHeader />
      <main>
        <Features />
        <BindingSection />
        <Mercury />
      </main>
    </Layout>
  );
}
