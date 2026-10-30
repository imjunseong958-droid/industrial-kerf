#include "editor_common.h"
#include "editor_theme.h"

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QPolygonF>

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>

namespace kerf {

bool nearerScrubFrame(double shownTime, bool hasShown, double frameTime, double target, double fps) {
    if (!hasShown) return true;
    const double slack = 0.5 / std::max(1.0, fps);
    return std::abs(frameTime - target) <= std::abs(shownTime - target) + slack;
}

QString validateInboundMcpConfig(const QByteArray &raw) {
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(raw, &error);
    if (error.error != QJsonParseError::NoError)
        return QStringLiteral("Invalid JSON at offset %1: %2").arg(error.offset).arg(error.errorString());
    if (!document.isObject()) return QStringLiteral("The JSON root must be an object.");
    const QJsonValue servers = document.object().value(QStringLiteral("mcpServers"));
    if (!servers.isObject()) return QStringLiteral("mcpServers must be an object.");
    const QJsonObject object = servers.toObject();
    for (auto it = object.begin(); it != object.end(); ++it) {
        if (!it.value().isObject()) return QStringLiteral("Each server must be an object.");
        const QJsonObject server = it.value().toObject();
        const QJsonValue command = server.value(QStringLiteral("command"));
        if (!command.isString() || command.toString().trimmed().isEmpty())
            return QStringLiteral("Each server needs a nonempty local command string. URL-only remote servers are not supported in Kerf chat.");
        for (const QString &field : {QStringLiteral("args"), QStringLiteral("tools")}) {
            if (!server.contains(field) || (field == QStringLiteral("tools") && server.value(field).isNull())) continue;
            if (!server.value(field).isArray()) return QStringLiteral("%1 must be an array of strings.").arg(field);
            for (const QJsonValue &value : server.value(field).toArray())
                if (!value.isString()) return QStringLiteral("%1 must contain only strings.").arg(field);
        }
        if (server.contains(QStringLiteral("env"))) {
            if (!server.value(QStringLiteral("env")).isObject()) return QStringLiteral("env must be an object of string values.");
            const QJsonObject env = server.value(QStringLiteral("env")).toObject();
            for (auto entry = env.begin(); entry != env.end(); ++entry)
                if (!entry.value().isString()) return QStringLiteral("env must contain only string values.");
        }
    }
    return {};
}

QString mcpExecutable(const QString &projectDirectory) {
    const QString override = qEnvironmentVariable("KERF_MCP_BIN");
    if (!override.isEmpty()) {
        return QFileInfo(override).isRelative() && !override.contains(QLatin1Char('/'))
            && !override.contains(QLatin1Char('\\'))
            ? override : QFileInfo(override).absoluteFilePath();
    }
#ifdef Q_OS_WIN
    const QString binary = QStringLiteral("kerf-mcp.exe");
#else
    const QString binary = QStringLiteral("kerf-mcp");
#endif
    QStringList candidates{QDir(QCoreApplication::applicationDirPath()).filePath(binary)};
    const QStringList roots{QCoreApplication::applicationDirPath(), QDir::currentPath(),
                            QFileInfo(QString::fromUtf8(__FILE__)).absolutePath(), projectDirectory};
    for (const QString &root : roots) {
        QDir directory(root);
        do {
            if (directory.exists(QStringLiteral("crates/kerf-mcp/Cargo.toml"))) {
                candidates << directory.filePath(QStringLiteral("target/release/") + binary)
                           << directory.filePath(QStringLiteral("target/debug/") + binary);
                break;
            }
        } while (directory.cdUp());
    }
    for (const QString &candidate : candidates) {
        const QFileInfo info(candidate);
        if (info.isFile() && info.isExecutable()) return info.absoluteFilePath();
    }
    return QStringLiteral("kerf-mcp");
}

QString prettyTime(double seconds) {
    seconds = std::max(0.0, seconds);
    const int minutes = static_cast<int>(seconds) / 60;
    const double remainder = seconds - minutes * 60.0;
    return QStringLiteral("%1:%2.%3")
        .arg(minutes, 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(remainder), 2, 10, QLatin1Char('0'))
        .arg(static_cast<int>(std::round((remainder - std::floor(remainder)) * 100.0)), 2, 10,
             QLatin1Char('0'));
}

QString timelineTimecode(double seconds, double fps) {
    const double rate = std::max(1.0, fps);
    const int frameRate = std::max(1, static_cast<int>(std::lround(rate)));
    const long long rounded = std::llround(std::max(0.0, seconds) * rate);
    const int frames = static_cast<int>(rounded % frameRate);
    long long whole = rounded / frameRate;
    const int secs = static_cast<int>(whole % 60);
    whole /= 60;
    const int mins = static_cast<int>(whole % 60);
    const int hours = static_cast<int>(whole / 60);
    return QStringLiteral("%1:%2:%3:%4")
        .arg(hours, 2, 10, QLatin1Char('0'))
        .arg(mins, 2, 10, QLatin1Char('0'))
        .arg(secs, 2, 10, QLatin1Char('0'))
        .arg(frames, 2, 10, QLatin1Char('0'));
}

QString timelineRulerLabel(double seconds, double fps, double duration) {
    const QString full = timelineTimecode(seconds, fps);
    return duration >= 3600.0 ? full : full.mid(3);
}

RulerScale rulerScale(double pixelsPerSecond, double fps) {
    const double pps = std::max(pixelsPerSecond, 0.001);
    const double frame = 1.0 / std::max(1.0, fps);
    const double steps[] = {frame,      2 * frame, 5 * frame, 10 * frame, 15 * frame, 1,    2,   5,
                            10,         15,        30,        60,         120,        300, 600, 1800,
                            3600,       7200};
    double major = steps[std::size(steps) - 1];
    for (double step : steps) {
        if (step * pps >= 72.0) {
            major = step;
            break;
        }
    }
    int divisions = 1;
    if (major > frame * 1.01) {
        for (int candidate : {10, 5, 4, 2}) {
            if ((major / candidate) * pps >= 7.0) {
                divisions = candidate;
                break;
            }
        }
    }
    return {major, std::max(1, divisions)};
}

QString clipName(const QJsonObject &clip) {
    if (clip.value(QStringLiteral("name")).isString()) {
        return clip.value(QStringLiteral("name")).toString();
    }
    const QString id = clip.value(QStringLiteral("id")).toString();
    return id.isEmpty() ? QStringLiteral("Untitled clip") : id.left(18);
}

QString sourceName(const QJsonObject &source) {
    const QString path = source.value(QStringLiteral("path")).toString();
    return path.isEmpty() ? QStringLiteral("Untitled media") : QFileInfo(path).fileName();
}

QSize resolvedCanvasSize(const QJsonObject &project, const QJsonArray &sources) {
    const auto validDimension = [](const QJsonValue &value) {
        const double number = value.toDouble();
        return std::isfinite(number) && number > 0.0;
    };
    const auto dimension = [](double value) {
        return static_cast<int>(std::round(std::clamp(
            value, 2.0, static_cast<double>(std::numeric_limits<int>::max()))));
    };
    if (validDimension(project.value(QStringLiteral("width"))) &&
        validDimension(project.value(QStringLiteral("height")))) {
        return QSize(dimension(project.value(QStringLiteral("width")).toDouble()),
                     dimension(project.value(QStringLiteral("height")).toDouble()));
    }
    QSize largest;
    double largestArea = 0.0;
    for (const auto &value : sources) {
        const QJsonObject source = value.toObject();
        if (!validDimension(source.value(QStringLiteral("width"))) ||
            !validDimension(source.value(QStringLiteral("height")))) continue;
        const double width = source.value(QStringLiteral("width")).toDouble();
        const double height = source.value(QStringLiteral("height")).toDouble();
        if (width * height > largestArea) {
            largestArea = width * height;
            largest = QSize(dimension(width), dimension(height));
        }
    }
    return largest.isValid() ? largest : QSize(1920, 1080);
}

QColor modelColor(const QString &hex) {
    QColor color(hex.left(7));
    if (hex.size() == 9) color.setAlpha(hex.mid(7).toInt(nullptr, 16));
    return color;
}

QString modelColorName(const QColor &color) {
    QString hex = color.name();
    if (color.alpha() != 255) hex += QStringLiteral("%1").arg(color.alpha(), 2, 16, QLatin1Char('0'));
    return hex;
}

bool isAudioTrack(const QJsonObject &track) {
    const QString type = track.value(QStringLiteral("type")).toString().toLower();
    return type.contains(QStringLiteral("audio")) || type.contains(QStringLiteral("sound"));
}

void frameInside(QPainter &p, const QRectF &rect, qreal thickness, const QColor &color) {
    p.fillRect(QRectF(rect.left(), rect.top(), rect.width(), thickness), color);
    p.fillRect(QRectF(rect.left(), rect.bottom() - thickness, rect.width(), thickness), color);
    p.fillRect(QRectF(rect.left(), rect.top() + thickness, thickness, rect.height() - 2 * thickness), color);
    p.fillRect(QRectF(rect.right() - thickness, rect.top() + thickness, thickness, rect.height() - 2 * thickness), color);
}

void blitLaneCache(QPainter &painter, const QPixmap &cache, const QRegion &clip) {
    const qreal dpr = std::max<qreal>(1, cache.devicePixelRatio());
    painter.save();
    painter.setClipRegion(clip, Qt::IntersectClip);
    for (const QRect &logical : clip) {
        const QRectF source(logical.x() * dpr, logical.y() * dpr, logical.width() * dpr, logical.height() * dpr);
        painter.drawPixmap(QRectF(logical), cache, source);
    }
    painter.restore();
}

QIcon transportIcon(TransportGlyph glyph) {
    QPixmap pixmap(32, 32);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(Qt::NoPen);
    painter.setBrush(editorTheme().glyph);
    const auto triangle = [&painter](qreal base, qreal tip) {
        painter.drawPolygon(QPolygonF{{base, 3.5}, {tip, 8.0}, {base, 12.5}});
    };
    switch (glyph) {
    case TransportGlyph::Start: painter.drawRect(QRectF(3.0, 3.5, 1.6, 9.0)); triangle(13.0, 5.5); break;
    case TransportGlyph::PreviousFrame: triangle(10.5, 3.5); painter.drawRect(QRectF(11.5, 3.5, 1.6, 9.0)); break;
    case TransportGlyph::Play: triangle(4.5, 12.5); break;
    case TransportGlyph::Pause: painter.drawRect(QRectF(4.5, 3.5, 2.4, 9.0)); painter.drawRect(QRectF(9.1, 3.5, 2.4, 9.0)); break;
    case TransportGlyph::NextFrame: painter.drawRect(QRectF(2.9, 3.5, 1.6, 9.0)); triangle(5.5, 12.5); break;
    case TransportGlyph::End: triangle(3.0, 10.5); painter.drawRect(QRectF(11.4, 3.5, 1.6, 9.0)); break;
    }
    return QIcon(pixmap);
}

QIcon maskToolIcon(MaskTool tool) {
    QPixmap pixmap(32, 32);
    pixmap.setDevicePixelRatio(2);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(editorTheme().glyph, 1.4));
    painter.setBrush(Qt::NoBrush);
    switch (tool) {
    case MaskTool::None: break;
    case MaskTool::Rectangle: painter.drawRect(QRectF(2.5, 3.5, 11.0, 9.0)); break;
    case MaskTool::Ellipse: painter.drawEllipse(QRectF(2.0, 3.0, 12.0, 10.0)); break;
    case MaskTool::Pen: {
        const QPolygonF corners{{2.5, 12.5}, {5.5, 3.0}, {13.0, 6.0}, {10.0, 13.0}};
        painter.drawPolygon(corners);
        painter.setPen(Qt::NoPen);
        painter.setBrush(editorTheme().glyph);
        for (const QPointF &corner : corners) painter.drawRect(QRectF(corner - QPointF(1.5, 1.5), QSizeF(3.0, 3.0)));
        break;
    }
    }
    return QIcon(pixmap);
}

double propertyAt(const QJsonObject &props, const QString &name, double clipTime, double fallback) {
    const QJsonValue value = props.value(name);
    if (value.isDouble()) return value.toDouble();
    const QJsonArray keys = value.toArray();
    if (keys.isEmpty()) return fallback;
    const auto at = [&keys](int index) { return keys.at(index).toObject().value(QStringLiteral("at")).toDouble(); };
    const auto valueOf = [&keys](int index) { return keys.at(index).toObject().value(QStringLiteral("value")).toDouble(); };
    if (clipTime <= at(0)) return valueOf(0);
    for (int index = 1; index < keys.size(); ++index) {
        if (clipTime > at(index)) continue;
        const double span = at(index) - at(index - 1);
        const double t = span > 0.0 ? (clipTime - at(index - 1)) / span : 1.0;
        return valueOf(index - 1) + (valueOf(index) - valueOf(index - 1)) * t;
    }
    return valueOf(keys.size() - 1);
}

QTransform picturePlacement(const QJsonObject &clip, const QSizeF &picture, bool fit, const QSizeF &canvas,
                            double clipTime) {
    // The compositor's geometry (kerf-compose `Scene::geometry`).
    const QJsonObject props = clip.value(QStringLiteral("props")).toObject();
    const auto prop = [&](const char *name, double fallback) {
        return propertyAt(props, QString::fromLatin1(name), clipTime, fallback);
    };
    const double l = prop("crop_l", 0.0), r = prop("crop_r", 0.0), t = prop("crop_t", 0.0), b = prop("crop_b", 0.0);
    const double sw = std::max(1.0, picture.width()), sh = std::max(1.0, picture.height());
    const double cw = canvas.width(), ch = canvas.height();
    const double width = sw * std::max(1e-6, 1.0 - l - r), height = sh * std::max(1e-6, 1.0 - t - b);
    double fw = width, fh = height;
    if (fit) {
        fw = std::max(1.0, std::min(std::round(ch * width / height), cw));
        fh = std::max(1.0, std::min(std::round(cw * height / width), ch));
    }
    const double scale = prop("scale", 1.0);
    const double kx = scale * prop("scale_x", 1.0), ky = scale * prop("scale_y", 1.0);
    const double ax = prop("anchor_x", 0.0), ay = prop("anchor_y", 0.0);
    QTransform placement;
    placement.translate(cw / 2.0 + prop("x", 0.0) * cw + ax * fw, ch / 2.0 + prop("y", 0.0) * ch + ay * fh);
    placement.rotate(prop("rotation", 0.0));
    placement.scale(fw / width * kx, fh / height * ky);
    placement.translate(-(sw * l + width * (0.5 + ax)), -(sh * t + height * (0.5 + ay)));
    placement.scale(sw, sh);
    return placement;
}

QPainterPath maskPath(const QJsonObject &mask) {
    QPainterPath path;
    const auto numbers = [&mask](const char *key) {
        const QJsonArray values = mask.value(QString::fromLatin1(key)).toArray();
        std::array<double, 4> out{};
        for (int index = 0; index < 4 && index < values.size(); ++index) out[index] = values.at(index).toDouble();
        return out;
    };
    if (mask.contains(QStringLiteral("rect"))) {
        const auto [left, top, right, bottom] = numbers("rect");
        path.addRect(QRectF(QPointF(left, top), QPointF(right, bottom)));
    } else if (mask.contains(QStringLiteral("ellipse"))) {
        const auto [x, y, rx, ry] = numbers("ellipse");
        path.addEllipse(QPointF(x, y), rx, ry);
    } else {
        const QJsonArray points = mask.value(QStringLiteral("points")).toArray();
        for (int index = 0; index < points.size(); ++index) {
            const QJsonArray point = points.at(index).toArray();
            const QPointF at(point.at(0).toDouble(), point.at(1).toDouble());
            if (index == 0) path.moveTo(at);
            else path.lineTo(at);
        }
        if (mask.value(QStringLiteral("closed")).toBool(true) && points.size() > 2) path.closeSubpath();
    }
    return path;
}

} // namespace kerf
