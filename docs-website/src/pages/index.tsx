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

function Prose({ children }: { children: ReactNode }) {
  return (
    <div className="row">
      <div className="col col--8 col--offset-2">{children}</div>
    </div>
  );
}

function Features() {
  return (
    <section>
      <div className="container">
        <div className="kicker">It&rsquo;s simple</div>
        <h2 className={styles.centeredHeading}>Events in, effects out.</h2>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>Integrate anything</h3>
            <p>
              A freddie program responds to whatever you can observe. A keypress,
              an app being foregrounded, which tab you are
              looking at, what devices are plugged in, or something you have
              not thought of yet.
            </p>
          </div>
          <div className="col col--4">
            <h3>There&rsquo;s no stopping me</h3>
            <p>
              Because we're integrating events from many sources, we can easily do crazy things, such as cloning the repository you're looking at without leaving GitHub.com, or muting your microphone on Google Meet (without finding the tab!)
            </p>
          </div>
          <div className="col col--4">
            <h3>Testable, understandable</h3>
            <p>
              All of this runs through one pure function: state and event in, new
              state and a list of effects out. Nothing is performed along the
              way, so a test is a call and an assertion.
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
        <div className="kicker">It&rsquo;s powerful</div>
        <h2 className={styles.centeredHeading}>For programmers, by programmers</h2>
        <Prose>
          <p>
            You do not configure freddie. You write a program with
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
        </Prose>
      </div>
    </section>
  );
}

function BindingSection() {
  return (
    <section>
      <div className="container">
        <div className="kicker">It&rsquo;s a kind of magic</div>
        <h2 className={styles.centeredHeading}>It&rsquo;s a kind of magic.</h2>
        <Prose>
          <p>
            A binding is a trigger and the handler it runs, written on the level
            where it applies. Say we want a volume layer, where <code>up</code>{' '}
            and <code>down</code> change the volume and the layer remembers what
            it set it to. The volume lives on the layer, because that is the only
            place it is used:
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{bindingExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>And the handler:</p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{handlerExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
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
        </Prose>
      </div>
    </section>
  );
}

function Mercury() {
  return (
    <section className="alt-background">
      <div className="container">
        <h2 className={styles.centeredHeading}>Mercury rising.</h2>
        <Prose>
          <p>
            This repository ships one program built with freddie,
            called <code>mercury</code>. It is macOS-only and it requires
            accessibility permissions. You should not expect it to fit your use
            case: it is here to be read, run, studied, forked, and modified.
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="bash">
              {`git clone https://github.com/freddiehg/freddie
  cd freddie
  cargo install --path crates/mercury
  mercury`}
            </CodeBlock>
          </div>
        </Prose>
        <Prose>
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
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="bash">
              {`mercury install     # start it at login
  mercury restart     # replace the running one
  mercury logs        # follow what it is doing`}
            </CodeBlock>
          </div>
        </Prose>
      </div>
    </section>
  );
}

function AreYouReady() {
  return (
    <section>
      <div className="container">
        <h2 className={styles.centeredHeading}>Are you ready, Freddie?</h2>
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
      title="freddie - build a bespoke control plane for your computer"
      description="freddie is a set of tools for building a bespoke control plane for your computer. A freddie program ingests a stream of events and produces a stream of effects."
    >
      <HomepageHeader />
      <main>
        <Features />
        <DontStopMeNow />
        <BindingSection />
        <Mercury />
        <AreYouReady />
      </main>
    </Layout>
  );
}
