import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';

import Heading from '@theme/Heading';
import styles from './index.module.css';
import BrowserOnly from '@docusaurus/BrowserOnly';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <BrowserOnly>

            {() => (
            <img
            src="/BugStalker/img/biglogo.png"
            alt="BugStalker logo"
            className={styles.overviewImage}
            />
            )}
        </BrowserOnly>

        <Heading as="h1" className="hero__title">

        {siteConfig.title}

        </Heading>

        <BrowserOnly>
              {() => (
                <img
                  src="/BugStalker/gif/overview.gif"
                  alt="Overview of BugStalker usage"
                  className={styles.overviewDemo}
                />
              )}
            </BrowserOnly>

        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.buttons}>
        <Link
                className="button button--secondary button--lg"
                to="/docs/overview">
                Debugger documentation
              </Link>
        </div>
      </div>
    </header>
  );
}


export default function Home() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title={`Hello from ${siteConfig.title}`}
      description="BugStalker, a modern Rust debugger">
      <HomepageHeader />
    </Layout>
  );
}