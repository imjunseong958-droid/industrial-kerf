#include "media_artwork.h"
#include "editor_common.h"
#include "editor_theme.h"

#include <QDir>
#include <QFileInfo>
#include <QJsonObject>
#include <QPainter>
#include <QPixmap>

namespace kerf {

MediaArtwork::MediaArtwork(QObject *parent) : QObject(parent) {
    connect(&m_process, &QProcess::finished, this, [this](int code, QProcess::ExitStatus) {
        if (code == 0) {
            const QImage result = QImage::fromData(m_process.readAllStandardOutput());
            if (!result.isNull()) {
                (m_current.wave ? waves : frames).insert(m_current.id, result);
                emit changed();
            }
        }
        m_timeout.stop();
        next();
    });
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) { m_timeout.stop(); next(); }
    });
    m_timeout.setSingleShot(true);
    connect(&m_timeout, &QTimer::timeout, &m_process, &QProcess::kill);
}

MediaArtwork::~MediaArtwork() {
    disconnect(&m_process, nullptr, this, nullptr);
    m_timeout.stop();
    m_process.kill();
    m_process.waitForFinished(1000);
}

void MediaArtwork::request(const QJsonArray &sources, const QString &directory) {
    for (const auto &value : sources) {
        const auto source = value.toObject();
        const QString id = source.value("id").toString();
        const QString path = QDir(directory).absoluteFilePath(source.value("path").toString());
        const QString key = id + path + QString::number(QFileInfo(path).lastModified().toMSecsSinceEpoch());
        if (m_requested.contains(key)) continue;
        m_requested.insert(key);
        frames.remove(id); waves.remove(id);
        m_queue.enqueue({id, path, false});
        if (source.value("hasAudio").toBool()) m_queue.enqueue({id, path, true});
    }
    if (m_process.state() == QProcess::NotRunning) next();
}

QIcon MediaArtwork::icon(const QString &id) const {
    if (frames.contains(id)) return QPixmap::fromImage(frames.value(id));
    // Drawn on each call in the theme in force, so nothing here goes stale
    // when the appearance changes.
    QPixmap tile(96, 54);
    tile.fill(editorTheme().audioTile);
    QPainter painter(&tile);
    if (waves.contains(id)) {
        // FFmpeg draws the wave in white, which vanishes on the light tile:
        // ink it in the tile's glyph colour.
        QImage wave = waves.value(id).convertToFormat(QImage::Format_ARGB32_Premultiplied);
        QPainter ink(&wave);
        ink.setCompositionMode(QPainter::CompositionMode_SourceIn);
        ink.fillRect(wave.rect(), editorTheme().audioTileGlyph);
        ink.end();
        painter.setOpacity(0.65);
        painter.drawImage(QRect(0, 10, 96, 34), wave);
    } else {
        drawWaveformSymbol(painter, QRectF(tile.rect()).center(), 24, editorTheme().audioTileGlyph);
    }
    return tile;
}

void MediaArtwork::next() {
    if (m_queue.isEmpty()) return;
    m_current = m_queue.dequeue();
    QStringList args{"-nostdin", "-v", "error", "-i", m_current.path};
    if (m_current.wave) {
        args << "-filter_complex" << "aformat=channel_layouts=mono,showwavespic=s=2048x48:colors=white:scale=sqrt";
    } else {
        args << "-vf" << "thumbnail,scale=160:90:force_original_aspect_ratio=decrease,pad=160:90:(ow-iw)/2:(oh-ih)/2";
    }
    args << "-frames:v" << "1" << "-an" << "-f" << "image2pipe" << "-vcodec" << "png" << "pipe:1";
    m_process.start(QStringLiteral("ffmpeg"), args);
    m_timeout.start(30000);
}

} // namespace kerf
