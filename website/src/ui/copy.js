
const CHECK = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12.5L9.5 17L19 7.5"/></svg>';

// The copy button next to the install commands copies every line, without
// the prompt.
export function initCopy() {
  const button = document.querySelector('.cmd-copy');
  const text = [...document.querySelectorAll('.cmd code')]
    .map((line) => [...line.childNodes].filter((n) => n.nodeType === 3).map((n) => n.textContent).join(''))
    .join('\n');
  const idle = button.innerHTML;
  let timer;
  const flash = (label, markup) => {
    clearTimeout(timer);
    button.innerHTML = markup;
    button.setAttribute('aria-label', label);
    timer = setTimeout(() => {
      button.innerHTML = idle;
      button.setAttribute('aria-label', 'Copy the commands');
    }, 1600);
  };
  button.addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(text);
      flash('Commands copied', CHECK);
    } catch {
      flash('Could not copy the commands', idle);
    }
  });
}
