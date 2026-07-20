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

const handlerExample = `fn louder<'a>(_ev: &KeyEvent, node: Node<VolumeLayerPath<'a>, ()>) -> MercuryEffect {
    let layer: &mut VolumeLayer = node.parent.get_mut();
    layer.volume = layer.volume + 10;
    MercuryEffect::SetVolume(layer.volume)
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
          used to build something much more powerful: a control plane for your computer.
        </p>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>Integrate the whole machine</h3>
            <p>
              Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do
              eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut
              enim ad minim veniam, quis nostrud exercitation ullamco laboris.
            </p>
          </div>
          <div className="col col--4">
            <h3>One place the decision is made</h3>
            <p>
              Duis aute irure dolor in reprehenderit in voluptate velit esse
              cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat
              cupidatat non proident, sunt in culpa qui officia deserunt.
            </p>
          </div>
          <div className="col col--4">
            <h3>Pure, and therefore knowable</h3>
            <p>
              Sed ut perspiciatis unde omnis iste natus error sit voluptatem
              accusantium doloremque laudantium, totam rem aperiam, eaque ipsa
              quae ab illo inventore veritatis et quasi architecto beatae.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function DontStopMeNow() {
  return (
    <section className="alt-background">
      <div className="container">
        <h2 className={styles.centeredHeading}>Don&rsquo;t stop me now.</h2>
        <p>
          You do not configure <code>freddie</code>. You write a program with
          it. There is no config file, no DSL, and no schema of somebody
          else&rsquo;s questions to answer.
        </p>
        <p>
          That distinction only matters at the edges, and the edges are where
          you end up. A configuration format is a fixed set of questions its
          author decided to ask, and it works right until you want something
          they did not anticipate. Then it stops. Your options become abusing
          whatever escape hatch exists, or bolting a second program onto the
          side to do the part the first one cannot, which is how a keyboard
          setup turns into three files that disagree about what state you are
          in.
        </p>
        <p>
          Here you fork it, build it, and run it. The whole thing ships as a
          repository you clone and a binary you compile, so your edits are Rust
          and <code>cargo build</code> is the deploy step. The state is a struct
          you declared. The handlers are functions you wrote. Adding an event
          source is adding a variant. Nothing has to be expressible in someone
          else&rsquo;s format before you can do it, so there is no ceiling to
          hit and nothing to work around when you reach it.
        </p>
        <p>
          It costs more than a config file for the simple cases, and it wants a
          toolchain and a rebuild. That is the trade. It is also a much smaller
          trade than it was two years ago: hand the crate to an LLM and describe
          the binding you want.
        </p>
      </div>
    </section>
  );
}

function BindingSection() {
  return (
    <section>
      <div className="container">
        <h2 className={styles.centeredHeading}>A kind of magic.</h2>
        <p>
          A binding is a trigger and the handler it runs, written on the level
          where it applies. Say we want a volume layer, where <code>up</code>{' '}
          and <code>down</code> change the volume and the layer remembers what
          it set it to. The volume lives on the layer, because that is the only
          place it is used:
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
        <p>
          That is most of what the developer experience amounts to: the derive
          writes the dispatch, and the types carry what dispatch already worked
          out so your handler never re-derives it. A trigger that reads key
          events cannot be hung on a tab event, because the narrowing is a{' '}
          <code>TryFrom</code> that fails to compile rather than a branch that
          fails at three in the morning.
        </p>
        <p>
          The loop is short too. <code>bacon restart</code> rebuilds and
          replaces the running daemon, so an edited binding is live without you
          touching a window, and <code>mercury logs</code> prints one record per
          dispatched event carrying the event, the effects it produced, and the
          resulting state. When something is bound wrong, the log already says
          what happened.
        </p>
      </div>
    </section>
  );
}

function EventsCanBeAnything() {
  return (
    <section className="alt-background">
      <div className="container">
        <h2 className={styles.centeredHeading}>Events can be anything.</h2>
        <p>
          A key going down is an event. So is an app coming to the front, a tab
          changing its URL, a display waking up, a microphone being plugged in,
          a timer going off, and a frame arriving on a socket from a program
          you wrote last week. The event type is an enum you own, and adding a
          source is adding a variant to it.
        </p>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>Keys, before the app sees them</h3>
            <p>
              The grab hands you every key with its modifiers first, which is
              the whole reason a remapper can exist at all.
            </p>
          </div>
          <div className="col col--4">
            <h3>Anything that can open a socket</h3>
            <p>
              A frame arriving on <code>127.0.0.1:3883</code> becomes an event
              like any other. That is how a Chrome extension tells{' '}
              <code>mercury</code> which tab you are looking at.
            </p>
          </div>
          <div className="col col--4">
            <h3>Hardware coming and going</h3>
            <p>
              Both directions count: a monitor connecting, a headset
              disconnecting, an app quitting out from under the layer that was
              bound to it.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function EffectsCanBeAnything() {
  return (
    <section>
      <div className="container">
        <h2 className={styles.centeredHeading}>Effects can be anything.</h2>
        <p>
          A handler does not do things. It returns a list of things to be done,
          and the run loop does them. That split is what keeps{' '}
          <code>state.handle</code> a pure function, and it means the list can
          hold whatever you are willing to write.
        </p>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>Emit a key</h3>
            <p>
              Swallow <code>caps</code> and send <code>esc</code>, or turn one
              chord into four. The keyboard is just another thing the program
              can drive.
            </p>
          </div>
          <div className="col col--4">
            <h3>Move windows and apps</h3>
            <p>
              Foreground an app, throw the focused window at the left half of
              the screen, retitle the menu bar, put an overlay up saying what is
              bound right now.
            </p>
          </div>
          <div className="col col--4">
            <h3>Run arbitrary code</h3>
            <p>
              Call an API, shell out, clone the repository whose page you are
              sitting on. An effect is a variant and the arm that performs
              it.
            </p>
          </div>
        </div>
        <p style={{ paddingTop: '1rem' }}>
          Nothing in that list runs during dispatch, so a test asserts on what
          came back rather than on what happened. The show goes on afterwards.
        </p>
      </div>
    </section>
  );
}

function Mercury() {
  return (
    <section className="alt-background">
      <div className="container">
        <h2 className={styles.centeredHeading}>Mercury rising.</h2>
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
        <p>
          Once you want it there every time, <code>mercury install</code>{' '}
          registers it to start at login, and <code>mercury uninstall</code>{' '}
          takes that back out. The rest of the verbs drive the running one:{' '}
          <code>restart</code> replaces it after a rebuild, <code>stop</code>{' '}
          ends it through the model so a command layer hands your modifiers
          back, and <code>status</code> and <code>logs</code> report on it
          without touching it.
        </p>
        <div className={styles.codeBlockWrap}>
          <CodeBlock language="bash">
            {`mercury install     # start it at login
mercury restart     # replace the running one
mercury logs        # follow what it is doing`}
          </CodeBlock>
        </div>
        <div className={styles.ctaContainer}>
          <p className={styles.ctaLine}>So, are you ready, Freddie?</p>
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
        <DontStopMeNow />
        <BindingSection />
        <EventsCanBeAnything />
        <EffectsCanBeAnything />
        <Mercury />
      </main>
    </Layout>
  );
}
