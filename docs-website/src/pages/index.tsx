import Link from '@docusaurus/Link';
import CodeBlock from '@theme/CodeBlock';
import Layout from '@theme/Layout';
import { useState, type ReactNode } from 'react';
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
pub struct ResizeLayer;`;

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
freddie_windows::watch(move |focused| {
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
  const [showMore, setShowMore] = useState(false);

  return (
    <section>
      <div className="container">
        <div className="kicker">It’s your computer</div>
        <h2 className={styles.centeredHeading}>Bend it to your will</h2>
        <div className={`row ${styles.doableGrid}`}>
          <Doable title="Send commands to your agent">
            Send a screenshot or the contents of your clipboard to an agent. Or, select all, cut, send to your agent (&ldquo;please fix&rdquo;), and paste the result, all without ever leaving your app.
          </Doable>
          <Doable title="Check out a branch from GitHub">
            Press one key to check out the branch you&rsquo;re reviewing on GitHub, open your editor, and open the changed files. Then press one key to jump back.
          </Doable>
          <Doable title="Jump to Google Meet">
            You&rsquo;re scrolling, but your boss asked you a question about the sales figures. How quickly can you find Google Meet? One key foregrounds it, wherever it is. No sweat.
          </Doable>
        </div>
        <div className="row">
          <Doable title="Rearrange windows automatically">
            Connect a monitor and have your application windows go back to where they belong, without any futzing on your part.
          </Doable>
          <Doable title="Respond to scheduled events">
            Nudge yourself to stop doomscrolling after 30 minutes or decrease your screen&rsquo;s brightness at sundown.
          </Doable>
          <Doable title="Dispatch links to specific profiles">
            Every link on your machine can go through freddie, so work GitHub links always open in your work Chrome profile.
          </Doable>
        </div>
        {showMore ? (
          <div className="row">
            <Doable title="Build a HUD for your events">
              Float a heads-up display over everything you do, showing your agents&rsquo; responses and whatever else you want to keep an eye on.
            </Doable>
            <Doable title="Watch your CI">
              Wire your build&rsquo;s failures into freddie and get nudged the moment a run goes red, without babysitting a browser tab.
            </Doable>
            <Doable title="Roll your own claw">
              Build your own claw on rigorous foundations. Construct something more complex than if this was built on a pile of scripts.
            </Doable>
          </div>
        ) : (
          <div className={styles.ctaContainer}>
            <button
              type="button"
              className="button button--secondary button--lg"
              onClick={() => setShowMore(true)}
            >
              There&rsquo;s more&hellip;
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

function Features() {
  return (
    <section>
      <div className="container">
        <div className="kicker">It’s the elm architecture</div>
        {/* The only breakable space is the one after the comma, so a narrow
            screen wraps there rather than mid-clause. */}
        <h2 className={styles.centeredHeading}>
          Events&nbsp;in, effects&nbsp;out
        </h2>
        <div className="row" style={{ paddingTop: '1.5rem' }}>
          <div className="col col--4">
            <h3>Effects in</h3>
            <p>
              A freddie program responds to any arbitrary event stream. Each is incorporated into the same state, so an event handler can make complex decisions that take into account multiple sources of information.
            </p>
          </div>
          <div className="col col--4">
            <h3>Pure, testable core</h3>
            <p>
              You write individual event handlers, and freddie assembles them into a larger function: events in, descriptions of effects out. This larger function is pure and side-effect free, and thus easily tested and easy to reason about.
            </p>
          </div>
          <div className="col col--4">
            <h3>Effects out</h3>
            <p>
              You can handle these effects however you like. Resize windows, foreground apps, send keys. Because we're writing code and your handlers receive access to the state, there really is no limit to what you can accomplish.
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
              src="https://www.youtube-nocookie.com/embed/eM3wmsUWsbo"
              title="A demo of freddie"
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
        <h2 className={styles.centeredHeading}>
          For&nbsp;programmers, by&nbsp;programmers
        </h2>
        <Prose>
          <p>
            Other programs for remapping keys are configuration-driven, and that makes it difficult or impossible to handle unanticipated use cases.
          </p>
          <p>
            Want to bind keys? That&rsquo;s fine, because these apps allow that. But, want your application windows to go back where they belong the moment you connect to a monitor? You&rsquo;re out of luck — that&rsquo;s a device event, not a keybinding, and these apps don&rsquo;t allow you to incorporate arbitrary streams of events.
          </p>
          <h3>That&rsquo;s a sign of a deeper problem</h3>
          <p>
            Want these keybindings to do different things in different states? Well, you&rsquo;d better hope that the app exposed that aspect of the state to you. Different keybindings for different active apps? That&rsquo;s doable, because it&rsquo;s anticipated and allowlisted. But, what about custom mute/unmute keybindings for when you&rsquo;re in an active Google Meet call? Not possible.
          </p>
          <p>
            And, in configuration-driven frameworks, you don&rsquo;t write functions, so your handlers don&rsquo;t get access to the state at all! Want one key that maximizes a window and, pressed again, puts it back exactly where it was? Then something has to remember the window&rsquo;s old position, in other words, it needs to be a function that is passed state.
          </p>
          <p>
            Now, more folks are willing to write configuration than to write and compile a Rust program. But guess what — freddie isn&rsquo;t for everyone. So, if you&rsquo;re willing to clone a repo, make some changes and run cargo build, freddie is here to give you incredible power.
          </p>
        </Prose>
      </div>
    </section>
  );
}

function BindingSection() {
  return (
    <section className="alt-background">
      <div className="container">
        <div className="kicker">It’s a kind of magic</div>
        <h2 className={styles.centeredHeading}>Handle complexity with ease</h2>
        <Prose>
          <p>
            The value of using a programming language and compiling our own program becomes apparent when we move beyond simple examples. Here, we&rsquo;ll build something that is impossible (or at least, awkward) in any other framework: the ability to maximize windows, and later restore them to their previous location.
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
          <h3>Populating the state</h3>
          <p>
            So far so good — we wrote handlers that accessed and mutated the state, and emitted effects that did the right thing. But that <code>focused</code> field did not fill itself in. We have to hook that up ourselves, too:
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
            <code>mercury</code> boots into the typing layer, which passes all
            keystrokes through. Typing <code>jk</code> takes you to the home layer.
            From there, <code>n</code> takes you to nav, <code>i</code> to in-app,{' '}
            <code>s</code> to per-site, and <code>r</code> to resize.{' '}
            <code>o</code> displays an overlay containing the layer&rsquo;s keymap.
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
        <DontStopMeNow />
        <Features />
        <BindingSection />
        <Video />
        <Mercury />
        <AreYouReady />
      </main>
    </Layout>
  );
}
