// Progressive enhancement only. Every number on every screen is already in the
// server-rendered HTML; this file draws the cumulative chart with uPlot and
// makes the upload dropzone a little nicer. No client-side state machine.
(function () {
  var chart = null;

  function cssVar(name, fallback) {
    var v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return v || fallback;
  }

  function rgba(hex, alpha) {
    var h = hex.replace('#', '');
    if (h.length === 3) {
      h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
    }
    var n = parseInt(h, 16);
    if (isNaN(n) || h.length !== 6) return 'rgba(31,111,84,' + alpha + ')';
    return 'rgba(' + ((n >> 16) & 255) + ',' + ((n >> 8) & 255) + ',' + (n & 255) + ',' + alpha + ')';
  }

  function initChart() {
    var el = document.getElementById('chart');
    var dataEl = document.getElementById('chart-data');
    if (!el || !dataEl || typeof uPlot === 'undefined') return;

    var data;
    try { data = JSON.parse(dataEl.textContent); } catch (e) { return; }
    if (!data || !data.x || !data.x.length) return;

    var dark = !!(window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches);
    var accent = cssVar('--accent', '#1f6f54');
    var neg = cssVar('--neg', '#b03a2e');
    var line = cssVar('--line', '#e5dfd3');
    var muted = cssVar('--muted', '#6d6960');
    var EXPONENT = 100; // EUR has two decimal places (see ADR-0003)
    var span = data.x[data.x.length - 1] - data.x[0];
    var showClock = span <= 2 * 86400;

    function major(values) {
      return values.map(function (n) { return n / EXPONENT; });
    }
    function fmt(v) { return v == null ? '--' : v.toFixed(2); }
    function gridColor() { return rgba(line, dark ? 0.75 : 0.9); }
    function tickLabel(t) {
      var iso = new Date(t * 1000).toISOString();
      return showClock ? iso.slice(11, 16) : iso.slice(0, 10);
    }

    var opts = {
      width: el.clientWidth || 900,
      height: 340,
      padding: [10, 16, 0, 0],
      legend: { show: false },
      cursor: { drag: { x: true, y: false } },
      series: [
        { label: 'Time' },
        {
          label: 'Winnings',
          stroke: accent,
          width: 2,
          fill: function (u) {
            var g = u.ctx.createLinearGradient(0, u.bbox.top, 0, u.bbox.top + u.bbox.height);
            g.addColorStop(0, rgba(accent, dark ? 0.28 : 0.16));
            g.addColorStop(1, rgba(accent, 0));
            return g;
          },
          value: function (u, v) { return fmt(v); }
        },
        {
          label: 'All-in adjusted',
          stroke: neg,
          width: 1.5,
          dash: [5, 4],
          value: function (u, v) { return fmt(v); }
        }
      ],
      axes: [
        {
          stroke: muted,
          grid: { stroke: gridColor(), width: 1 },
          ticks: { stroke: rgba(line, dark ? 0.9 : 0.9), width: 1 },
          values: function (u, ticks) { return ticks.map(tickLabel); }
        },
        {
          stroke: muted,
          size: 62,
          grid: { stroke: gridColor(), width: 1 },
          ticks: { stroke: rgba(line, dark ? 0.9 : 0.9), width: 1 },
          values: function (u, ticks) { return ticks.map(fmt); }
        }
      ],
      scales: { x: { time: true } }
    };

    if (chart) {
      chart.destroy();
      chart = null;
    }
    chart = new uPlot(opts, [data.x, major(data.net), major(data.adjusted)], el);

    if (window.ResizeObserver) {
      if (el._textureRo) el._textureRo.disconnect();
      var ro = new ResizeObserver(function () {
        if (!chart || !el.isConnected) return;
        var width = el.clientWidth;
        if (width > 0) chart.setSize({ width: width, height: 340 });
      });
      ro.observe(el);
      el._textureRo = ro;
    }
  }

  function wireDropzones() {
    var zones = document.querySelectorAll('.dropzone');
    for (var i = 0; i < zones.length; i++) {
      var zone = zones[i];
      if (zone.dataset.wired === '1') continue;
      zone.dataset.wired = '1';

      var input = zone.querySelector('input[type="file"]');
      if (!input) continue;

      ['dragenter', 'dragover'].forEach(function (name) {
        zone.addEventListener(name, function (e) {
          e.preventDefault();
          zone.classList.add('dragging');
        });
      });
      ['dragleave', 'dragend'].forEach(function (name) {
        zone.addEventListener(name, function () { zone.classList.remove('dragging'); });
      });
      zone.addEventListener('drop', function (e) {
        e.preventDefault();
        zone.classList.remove('dragging');
        if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length) {
          input.files = e.dataTransfer.files;
          input.dispatchEvent(new Event('change', { bubbles: true }));
        }
      });
      input.addEventListener('change', function () {
        var hint = zone.querySelector('.drop-hint');
        if (!hint) return;
        if (input.files && input.files.length) {
          var names = [];
          for (var j = 0; j < input.files.length && j < 4; j++) names.push(input.files[j].name);
          hint.textContent = names.join(', ') + (input.files.length > 4 ? ' +' + (input.files.length - 4) + ' more' : '');
        }
      });
    }
  }

  function boot() {
    initChart();
    wireDropzones();
  }

  document.addEventListener('DOMContentLoaded', boot);
  document.body.addEventListener('htmx:afterSwap', boot);

  // Follow an OS theme change without a reload: rebuild the canvas with the
  // new custom-property values.
  if (window.matchMedia) {
    var media = window.matchMedia('(prefers-color-scheme: dark)');
    var onChange = function () { initChart(); };
    if (media.addEventListener) media.addEventListener('change', onChange);
    else if (media.addListener) media.addListener(onChange);
  }
})();
