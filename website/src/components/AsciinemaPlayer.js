import React, { useEffect, useRef, useState } from 'react';

export default function AsciinemaPlayer({ src, options = {} }) {
  const ref = useRef(null);
  const [isReady, setIsReady] = useState(false);

  useEffect(() => {
    if (typeof window === 'undefined') return;

    const cssId = 'asciinema-player-css';
    if (!document.getElementById(cssId)) {
      const css = document.createElement('link');
      css.id = cssId;
      css.rel = 'stylesheet';
      css.href = 'https://cdn.jsdelivr.net/npm/@asciinema/player@3.0.0/dist/themes/asciinema-player.css';
      document.head.appendChild(css);
    }

    const scriptId = 'asciinema-player-script';
    let script = document.getElementById(scriptId);
    
    if (!script) {
      script = document.createElement('script');
      script.id = scriptId;
      script.src = 'https://cdn.jsdelivr.net/npm/@asciinema/player@3.0.0/dist/asciinema-player.min.js';
      script.async = true;
      document.body.appendChild(script);
    }

    const initPlayer = () => {
      if (!window.AsciinemaPlayer) {
        console.error('AsciinemaPlayer not available');
        return;
      }

      const playerSrc = src.startsWith('http')
        ? src
        : `${window.location.origin}${src}`;
      
      try {
        window.AsciinemaPlayer.create(playerSrc, ref.current, {
          cols: 120,
          rows: 24,
          autoPlay: true,
          fit: 'width',
          ...options
        });
        setIsReady(true);
      } catch (error) {
        console.error('Player initialization failed:', error);
      }
    };

    if (window.AsciinemaPlayer) {
      initPlayer();
    } else {
      script.onload = initPlayer;
    }

    return () => {
      if (ref.current) {
        ref.current.innerHTML = '';
      }
    };
  }, [src, options]);

  return (
    <div ref={ref} style={{
      minHeight: '300px',
      backgroundColor: isReady ? 'transparent' : '#f5f5f5',
      borderRadius: '4px',
      margin: '20px 0',
      position: 'relative'
    }}>
      {!isReady && (
        <div style={{
          position: 'absolute',
          top: '50%',
          left: '50%',
          transform: 'translate(-50%, -50%)',
          color: '#666'
        }}>
          Loading player...
        </div>
      )}
    </div>
  );
}