const beans = ['C', 'O', 'O', 'L', 'B', 'E', 'A', 'N', 'S'];
const cm = document.querySelector('[confirm-modal]');
const ctn = document.querySelector('[container]');

let i = 0;
let bean_count = 5;
setInterval(() => {
  let og_bean = beans[i % beans.length];
  beans[i % beans.length] = beans[i % beans.length].toLowerCase();
  window.document.title = `🧊 ${beans.join(' ')}`;
  beans[i % beans.length] = og_bean;
  i++;
}, 1000);