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
            <h3>A binding can use anything you know</h3>
            <p>
              Every source feeds the same state, so what a key does can depend
              on all of it at once. That is how <code>n</code> opens a new chat
              while you are on claude.ai and means nothing anywhere else. A
              remapper that can only see the frontmost app has no way to say
              that.
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
            <h3>Pure, and therefore knowable</h3>
            <p>
              <code>state.handle(event)</code> takes state and event and hands
              back the updated state and a list of effects, performing none of
              them. What a key does in a given layer is something you read off
              rather than something you run the program to find out. Every
              state a binding can be reached in is written down, so the
              question of what happens next has an answer you can check without
              a keyboard in your hand. A test asserts on exactly that.
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
            <h3>Under pressure</h3>
            <p>
              The keyboard grab hands you every key with its modifiers before
              the frontmost app sees any of it, which is the whole reason a
              remapper can exist.
            </p>
          </div>
          <div className="col col--4">
            <h3>Radio ga ga</h3>
            <p>
              Anything that can open a socket to <code>127.0.0.1:3883</code> is
              an event source. That is how a Chrome extension tells{' '}
              <code>mercury</code> which tab you are looking at.
            </p>
          </div>
          <div className="col col--4">
            <h3>Another one bites the dust</h3>
            <p>
              Hardware comes and goes, and both directions are events. A monitor
              connecting, a headset disconnecting, an app quitting out from
              under the layer that was bound to it.
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
            <h3>Hammer to fall</h3>
            <p>
              Emit a key. Swallow <code>caps</code> and send <code>esc</code>,
              or turn one chord into four. The keyboard is just another thing
              the program can drive.
            </p>
          </div>
          <div className="col col--4">
            <h3>Play the game</h3>
            <p>
              Foreground an app, throw the focused window at the left half of
              the screen, retitle the menu bar, put an overlay up saying what is
              bound right now.
            </p>
          </div>
          <div className="col col--4">
            <h3>I want it all</h3>
            <p>
              Run arbitrary code. Call an API, shell out, clone the repository
              whose page you are sitting on. An effect is a variant and the arm
              that performs it.
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
