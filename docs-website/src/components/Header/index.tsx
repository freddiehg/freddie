import Buttons from '../Buttons';
import styles from './styles.module.css';

export default function HomepageHeader() {
  return (
    <header className={`hero hero--primary ${styles.heroBanner}`}>
      <div className="container">
        <div className="row">
          <div className="col">
            <img
              className={styles.heroLogo}
              src="/img/freddie.png"
              alt="freddie"
            />
            <h1 className={styles.heroTitle}>freddie</h1>
            <p
              className={`hero__subtitle margin-bottom--lg ${styles.heroSubtitle}`}
            >
              Build a bespoke control plane for your computer.
            </p>
            <Buttons />
          </div>
        </div>
      </div>
    </header>
  );
}
