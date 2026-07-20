import Link from '@docusaurus/Link';
import styles from './styles.module.css';

export default function Buttons() {
  return (
    <div className={styles.buttons}>
      <Link
        className="button button--secondary button--lg"
        to="/docs/getting-started-with-mercury"
      >
        Get started
      </Link>
      <Link
        className="button button--secondary button--lg"
        to="https://github.com/freddiehg/freddie"
      >
        GitHub
      </Link>
    </div>
  );
}
