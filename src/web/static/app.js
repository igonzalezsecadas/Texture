// Progressive enhancement only. Every number on every screen is already in the
// server-rendered HTML; this file draws the cumulative chart with uPlot.
// Money arrives in minor units so no arithmetic happens in JavaScript beyond a
// display-only division by the currency exponent.
(function () {
  var el = document.getElementById('chart');
  var dataEl = document.getElementById('chart-data');
  if (!el || !dataEl || typeof uPlot === 'undefined') return;
  var data;
  try { data = JSON.parse(dataEl.textContent); } catch (e) { return; }
  if (!data || !data.x || !data.x.length) return;

  var EXPONENT = 100; // EUR has two decimal places (see ADR-0003)
  function major(values) {
    return values.map(function (n) { return n / EXPONENT; });
  }
  function fmt(v) { return v == null ? '--' : v.toFixed(2); }

  var opts = {
    width: el.clientWidth || 900,
    height: 320,
    series: [
      { label: 'Time' },
      { label: 'Winnings', stroke: '#2481cc', width: 2, value: function (u, v) { return fmt(v); } },
      { label: 'All-in adjusted', stroke: '#c0392b', width: 1, dash: [6, 4], value: function (u, v) { return fmt(v); } }
    ],
    axes: [
      { values: function (u, ticks) {
          return ticks.map(function (t) { return new Date(t * 1000).toISOString().slice(0, 10); });
        } },
      { values: function (u, ticks) { return ticks.map(fmt); } }
    ],
    scales: { x: { time: true } }
  };
  new uPlot(opts, [data.x, major(data.net), major(data.adjusted)], el);
})();
