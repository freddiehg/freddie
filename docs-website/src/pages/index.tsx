import Link from '@docusaurus/Link';
import CodeBlock from '@theme/CodeBlock';
import Layout from '@theme/Layout';
import type { ReactNode } from 'react';
import HomepageHeader from '../components/Header';
import styles from './index.module.css';

const stateExample = `pub struct Mercury {
    /// The focused window and where it sits.
    focused: Option<(WindowId, Frame)>,
    /// Where each window was before we moved it.
    prior_locations: HashMap<WindowId, Frame>,

    #[resolve_into]
    layer: Layer,
}`;

const bindingExample = `#[derive(Bind, Debug)]
#[node(parent = LayerPath)]
#[binds(MercuryStruct)]
#[bind(
    Key::UpArrow.down() => maximize,
    Key::KeyR.down() => restore,
)]
pub struct ResizeLayer {}`;

const maximizeExample = `fn maximize<'a>(
    _ev: &KeyEvent,
    node: Node<ResizeLayerPath<'a>, ()>,
) -> Option<MercuryEffect> {
    // ResizeLayer -> Layer -> Mercury, where the frames are kept.
    let root: &mut Mercury = node.parent.ascend();

    let (id, frame) = root.focused?;
    // Only the first maximize records anything. A second one
    // finds the entry already there and leaves it alone, so
    // \`r\` still goes back to where the window started.
    root.prior_locations.entry(id).or_insert(frame);

    Some(MercuryEffect::Place(Placement::Maximize))
}`;

const handlerExample = `fn restore<'a>(
    _ev: &KeyEvent,
    node: Node<ResizeLayerPath<'a>, ()>,
) -> Option<MercuryEffect> {
    let root: &mut Mercury = node.parent.ascend();

    let (id, _) = root.focused?;
    let frame = root.prior_locations.remove(&id)?;

    Some(MercuryEffect::Place(Placement::Exactly(frame)))
}`;

const eventExample = `pub enum MercuryEvent {
    Key(KeyEvent),
    Foreground(ForegroundEvent),
    Timer(TimerFired),
    /// Which window is focused and where it sits. New.
    Window(WindowFocused),
}`;

const trackExample = `#[bind(
    AnyWindowFocused => track_focus,
)]
pub struct MercuryStruct;

fn track_focus(
    ev: &WindowFocused,
    node: Node<MercuryPath, ()>,
) -> Option<MercuryEffect> {
    node.parent.get_mut().focused = Some((ev.window, ev.frame));
    None
}`;

const installExample = `git clone https://github.com/freddiehg/freddie
cd freddie
cargo install --path crates/mercury
mercury`;

const verbsExample = `mercury install     # start it at login
mercury restart     # replace the running one
mercury logs        # follow what it is doing`;

const sourceExample = `// In \`mercury daemon\`, beside the other sources.
freddie_windows::watch(move |window, frame| {
    let focused = WindowFocused { window, frame };
    let _ = events.send(MercuryEvent::Window(focused));
});`;

function Prose({ children }: { children: ReactNode }) {
  return (
    <div className="row">
      <div className="col col--8 col--offset-2">{children}</div>
    </div>
  );
}

function Doable({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className={`col col--4 ${styles.doable}`}>
      <h3 className={styles.doableTitle}>{title}</h3>
      <p>{children}</p>
    </div>
  );
}

function BendIt() {
  return (
    <section>
      <div className="container">
        <div className="kicker">It’s your computer</div>
        <h2 className={styles.centeredHeading}>Bend it to your will</h2>
        <div className={`row ${styles.doableGrid}`}>
          <Doable title="Remap keys however you like">
            Remap keys. Bind them differently in different layers and in different states. Want layers, sequences, or chords? Go for it. Bind whatever you want, wherever you want, however you want.
          </Doable>
          <Doable title="Clone the repo you&rsquo;re viewing">
            From a repo’s page, use one key to clone it, drop you into it, and open your editor. In today's fast paced environment, who has time to copy the URL, or switch to the terminal, and type the same four commands?
          </Doable>
          <Doable title="Mute Google Meet from anywhere">
            A global keybinding can mute Google Meet from anywhere. No more scrounging around to find the tab when you receive a phone call.
          </Doable>
        </div>
        <div className="row">
          <Doable title="Rearrange windows automatically">
            Connect a monitor and your windows go back where they belong.
          </Doable>
          <Doable title="Keys that remember">
            Maximize a window, press the same key again, and it returns to
            exactly the size and position it had. Something had to remember
            where that was, and something did.
          </Doable>
          <Doable title="Anything can talk to it">
            A browser extension, a build that just finished, whatever you wrote
            last week. If it can open a socket it can hand you an event, and a
            binding can be waiting for it.
          </Doable>
        </div>
      </div>
    </section>
  );
}

function Features() {
  return (
    <section className="alt-background">
      <div className="container">
        <div className="kicker">It’s simple</div>
        <h2 className={styles.centeredHeading}>Events in, effects out</h2>
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
            <h3>There’s no stopping me</h3>
            <p>
              Because we're integrating events from many sources, we can easily do crazy things, such as cloning the repository you're looking at without leaving GitHub.com, or muting your microphone on Google Meet (without finding the tab!)
            </p>
          </div>
          <div className="col col--4">
            <h3>Testable, understandable</h3>
            <p>
              A freddie program is centered around a single, pure function: state and event in, new
              state and a list of effects out. And so a freddie program remains testable and easy to reason about, even as it scales and grows more complex.
            </p>
          </div>
        </div>
      </div>
    </section>
  );
}

function Video() {
  return (
    <section>
      <div className="container">
        <div className="kicker">It’s worth a look</div>
        <h2 className={styles.centeredHeading}>See it running</h2>
        <div className="row">
          <div className="col col--8 col--offset-2 margin-top--md">
            <iframe
              width="100%"
              height="444"
              src="https://www.youtube-nocookie.com/embed/3GWZ9yiskFk"
              title="Placeholder video"
              allow="autoplay; clipboard-write; encrypted-media; picture-in-picture; web-share"
              allowFullScreen
              frameBorder="0"
            ></iframe>
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
        <div className="kicker">It’s powerful</div>
        <h2 className={styles.centeredHeading}>For programmers, by programmers</h2>
        <Prose>
          <p>
            Other programs for remapping keys are great, but they tend to be configuration-driven, and that makes it difficult or impossible to handle unanticipated use cases.
          </p>
          <p>
            Want to bind keys? That's fine, because these apps allow that. But, want your windows to go back where they belong the moment you connect to a monitor? You're out of luck — that's a device event, not a keybinding, and these apps don't allow you to incorporate arbitrary streams of events.
          </p>
          <p>
            And there's a deeper problem: want these keybindings to do different things in different states? Well, you'd better hope that the app exposed that aspect of the state to you. Different keybindings for different active apps? That's doable, because it's anticipated and allowlisted. But, what about custom mute/unmute keybindings for when you're in an active Google Meet call? Not possible.
          </p>
          <p>
            And, in configuration-driven frameworks, you don't write functions, so your handlers don't get access to the state at all! Want one key that maximizes a window and, pressed again, puts it back exactly where it was? Then something has to remember the window's old position, in other words, it needs to be a function that is passed state.
          </p>
          <p>
            Now, more folks are willing to write configuration than to write and compile a Rust program. But guess what — freddie isn't for everyone. So, if you're willing to clone a repo, make some changes and run cargo build, freddie is here to give you incredible power. (And if you're not? Just ask an LLM to do it!)
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
        <div className="kicker">It’s a kind of magic</div>
        <h2 className={styles.centeredHeading}>Handle complexity with ease</h2>
        <Prose>
          <p>
            The value of using a programming language and compiling our own program becomes apparent when we move beyond simple examples. Here, we'll build something that is impossible (or at least, awkward) in any other framework: the ability to maximize windows, and the ability to restore them to their previous location.
          </p>
          <p>
            First, we add the appropriate pieces of state onto our root struct:
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{stateExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>
            A binding is a trigger and the handler it runs, written on the layer
            where it applies. Up maximizes the focused window, and{' '}
            <code>r</code> puts it back.
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{bindingExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>Maximizing writes down where the window was:</p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{maximizeExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>And restoring reads it back out:</p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{handlerExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <h3>But wait &mdash; how did that state get there?</h3>
          <p>
            Amazing! And, simple, even. We wrote handlers that accessed and mutated the state, and emitted effects that did the right thing. But that <code>focused</code> field did not fill itself in. We have to hook that up ourselves, too:
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{eventExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>
            Something has to make one. A source is a stream you subscribe to,
            turning whatever it hands you into that variant:
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{sourceExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>
            And a binding at the root keeps the field current. It changes state
            and asks for nothing, so it returns <code>None</code>.
          </p>
        </Prose>
        <Prose>
          <div className={styles.codeBlockWrap}>
            <CodeBlock language="rust">{trackExample}</CodeBlock>
          </div>
        </Prose>
        <Prose>
          <p>
            Dispatch narrows an event to the kind a trigger reads before asking
            whether it matches, so the key bindings above never see a window
            event and did not have to be told this one exists.
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
        <div className="kicker">It’s ready for you</div>
        <h2 className={styles.centeredHeading}>Give mercury a try</h2>
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
            <CodeBlock language="bash">{installExample}</CodeBlock>
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
            <CodeBlock language="bash">{verbsExample}</CodeBlock>
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
        <div className="kicker">It’s time</div>
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
        <BendIt />
        <Features />
        <Video />
        <DontStopMeNow />
        <BindingSection />
        <Mercury />
        <AreYouReady />
      </main>
    </Layout>
  );
}
