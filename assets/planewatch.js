mapboxgl.accessToken = 'pk.eyJ1IjoidXR0ZXJzdGVwIiwiYSI6ImNsbm40ZDUydzAyaHUyaXAxMmdwaTFianAifQ.1Is8_jQQFdwfFrANHEhGzA';
const map = new mapboxgl.Map({
    container: 'map',
    style: 'mapbox://styles/mapbox/streets-v11',
    zoom: 8,
    center: [44.7781, 41.7051]
});

const hashCode = s => s.split('').reduce((a,b) => (((a << 5) - a) + b.charCodeAt(0))|0, 0)
const COLORS = ['#50BFE6','#EE34D2','#FD5B78','#FF00CC','#FF355E','#FF6037','#8FD400','#DA2C43','#6F2DA8','#FF6EFF','#FF3855','#FD3A4A','#FB4D46','#FA5B3D','#FFAA1D','#299617','#2243B6','#5DADEC','#5946B2','#9C51B6','#A83731','#AF6E4D','#FF5470','#FF7A00','#0048BA','#FF007C','#E936A7'];

const pointsHistory = await fetch("/points_history").then(r => r.json());
const pointsOverlay = document.getElementById("points-overlay");
const pointsOverlayCtx = pointsOverlay.getContext("2d");

const devicePixelRatio = window.devicePixelRatio || 1;

function placePoint(mode_s, [lat, long], map, overlay, overlayCtx) {
    const {x, y} = map.project([long, lat]);

    if (x > 0 && y > 0 && x < overlay.width && y < overlay.height) {
        overlayCtx.fillStyle = COLORS[hashCode(mode_s) % COLORS.length];
        overlayCtx.fillRect((x - 1) * devicePixelRatio, (y - 1) * devicePixelRatio, 2 * devicePixelRatio, 2 * devicePixelRatio);
    }
}

function drawFull(map, overlay, overlayCtx, points) {
    overlayCtx.clearRect(0, 0, overlay.width, overlay.height);

    const w = overlay.offsetWidth;
    const h = overlay.offsetHeight

    overlay.width = w * devicePixelRatio;
    overlay.height = h * devicePixelRatio;

    overlay.setAttribute("style", `width: ${w}px; height: ${h}px`);

    for (const [mode_s, point] of points) {
        placePoint(mode_s, point, map, overlay, overlayCtx);
    }
}

drawFull(map, pointsOverlay, pointsOverlayCtx, pointsHistory);

window.addEventListener('resize', () => {
    drawFull(map, pointsOverlay, pointsOverlayCtx, pointsHistory);
}, false);
window.addEventListener("orientationchange", function() {
    window.dispatchEvent(new Event("resize"));
}, false);
map.on('move', () => {
    drawFull(map, pointsOverlay, pointsOverlayCtx, pointsHistory);
});

const wsProto = window.location.protocol === 'https:' ? 'wss:' : 'ws';

const ws = new WebSocket(`${wsProto}//${window.location.host}/ws`);
ws.addEventListener('message', (event) => {
    function markerElement(isNew, mode_s) {
        const pointsMarker = document.createElement("div");
        pointsMarker.classList.add("marker");
        if (isNew) {
            pointsMarker.classList.add("marker-new");
        }
        pointsMarker.style.width = "4px";
        pointsMarker.style.height = "4px";
        pointsMarker.style.borderRadius = "4px";
        pointsMarker.style.background = "#F22";

        return pointsMarker;
    }

    const [mode_s, point] = JSON.parse(event.data);

    if (isNaN(point[0]) || isNaN(point[1])) {
        return;
    }

    pointsHistory.push([mode_s, point]);
    placePoint(mode_s, point, map, pointsOverlay, pointsOverlayCtx);

    const pointsMarker = markerElement(true);
    const marker = new mapboxgl.Marker(pointsMarker);
    marker.setLngLat({
        lng: point[1],
        lat: point[0],
    }).addTo(map);

    setTimeout(() => {
        marker.remove();
    }, 1000);
});

const cameraOverlay = document.getElementById("camView");
const cameraClose = document.getElementById("camViewClose");

window.addEventListener("keypress", (event) => {
    if (event.key === "c") {
        camView.showModal();
    }
}, false);

cameraClose.addEventListener("click", () => {
    camView.close();
}, false);
