#include "player_overlay.h"

namespace kerf {

void PlayerOverlay::setTime(double time, double duration, double fps) {
    m_time = time;
    m_duration = duration;
    m_fps = fps > 0 ? fps : 24.0;
}

} // namespace kerf
