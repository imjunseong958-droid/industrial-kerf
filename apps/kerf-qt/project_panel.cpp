#include "project_panel.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"

#include <QFontMetricsF>
#include <QJsonObject>
#include <QListWidget>
#include <QPainter>
#include <QVBoxLayout>

#include <utility>

namespace kerf {

namespace {

constexpr QSize kThumbnail{112, 63};

const QString kOffline = QStringLiteral("Offline");
const QString kNotDownloaded = QStringLiteral("Not downloaded");
const QString kExternal = QStringLiteral("External");

// A thumbnail letterboxed onto a fixed tile, so the bin keeps a grid, with
// the media's badge in its corner. Sound is a teal tile with its waveform
// (or a note until one arrives); a picture not yet drawn, or offline, is a
// quiet tile with an edge, so it never reads as a gap in the grid.
QIcon tile(const QImage &frame, const QImage &wave, bool audio, const QString &badge) {
    const EditorTheme &theme = editorTheme();
    QPixmap pixmap(kThumbnail);
    QPainter painter(&pixmap);
    if (!frame.isNull() && badge != kOffline) {
        pixmap.fill(theme.thumbnail);
        const QImage scaled = frame.scaled(kThumbnail, Qt::KeepAspectRatio, Qt::SmoothTransformation);
        painter.drawImage((kThumbnail.width() - scaled.width()) / 2, (kThumbnail.height() - scaled.height()) / 2, scaled);
    } else if (audio && badge != kOffline) {
        pixmap.fill(theme.audioTile);
        if (!wave.isNull()) {
            // FFmpeg draws the wave in white: ink it in the tile's glyph colour.
            QImage ink = wave.convertToFormat(QImage::Format_ARGB32_Premultiplied);
            QPainter tint(&ink);
            tint.setCompositionMode(QPainter::CompositionMode_SourceIn);
            tint.fillRect(ink.rect(), theme.audioTileGlyph);
            tint.end();
            painter.drawImage(QRect(0, 14, kThumbnail.width(), kThumbnail.height() - 28), ink);
        } else {
            QFont font = painter.font();
            font.setPixelSize(24);
            painter.setFont(font);
            painter.setPen(theme.audioTileGlyph);
            painter.drawText(pixmap.rect(), Qt::AlignCenter, QStringLiteral("♪"));
        }
    } else {
        pixmap.fill(theme.tileEmpty);
        painter.setPen(theme.tileEmptyBorder);
        painter.drawRect(pixmap.rect().adjusted(0, 0, -1, -1));
        QFont font = painter.font();
        font.setPixelSize(11);
        painter.setFont(font);
        painter.setPen(theme.tileEmptyText);
        painter.drawText(pixmap.rect(), Qt::AlignCenter, audio ? QStringLiteral("Audio") : QStringLiteral("Media"));
    }
    if (!badge.isEmpty()) {
        // Sentence case at 11 points, as macOS labels.
        QFont font = painter.font();
        font.setPixelSize(11);
        font.setWeight(QFont::DemiBold);
        painter.setFont(font);
        const QSizeF text = QFontMetricsF(font).size(Qt::TextSingleLine, badge);
        const QRectF box(4, 4, text.width() + 12, text.height() + 4);
        const QColor fill = badge == kOffline ? theme.badgeError : badge == kNotDownloaded ? theme.badgeInfo : theme.badgeNeutral;
        const QColor ink = badge == kOffline ? theme.badgeErrorText : badge == kNotDownloaded ? theme.badgeInfoText : theme.badgeNeutralText;
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        painter.setBrush(fill);
        painter.drawRoundedRect(box, 4, 4);
        painter.setPen(ink);
        painter.drawText(box, Qt::AlignCenter, badge);
    }
    return QIcon(pixmap);
}

QString badgeTip(const QString &badge) {
    if (badge == kOffline) return QStringLiteral("Offline: the file is missing. Replace Media… points this slot at another file.");
    if (badge == kNotDownloaded) return QStringLiteral("Not downloaded: a Git LFS file that a sync downloads.");
    if (badge == kExternal) return QStringLiteral("Outside the project folder, or ignored by Git there, so collaborators do not have it. Consolidate Media copies it into media/.");
    return {};
}

} // namespace

ProjectPanel::ProjectPanel(MediaArtwork *artwork, QWidget *parent) : QWidget(parent), m_artwork(artwork) {
    setObjectName(QStringLiteral("projectPanel"));
    setAttribute(Qt::WA_StyledBackground);
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(4, 4, 4, 4);
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("projectBin"));
    m_list->setViewMode(QListView::IconMode);
    m_list->setIconSize(kThumbnail);
    m_list->setGridSize(QSize(kThumbnail.width() + 16, kThumbnail.height() + 46));
    m_list->setTextElideMode(Qt::ElideMiddle);
    m_list->setResizeMode(QListView::Adjust);
    m_list->setMovement(QListView::Static);
    m_list->setWordWrap(true);
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
    layout->addWidget(m_list);
    connect(m_list, &QListWidget::itemActivated, this, [this](QListWidgetItem *item) {
        emit openRequested(item->data(Qt::UserRole).toString());
    });
    connect(m_list, &QListWidget::customContextMenuRequested, this, [this](const QPoint &at) {
        QListWidgetItem *item = m_list->itemAt(at);
        if (item) m_list->setCurrentItem(item);
        emit menuRequested(item ? item->data(Qt::UserRole).toString() : QString(), m_list->viewport()->mapToGlobal(at));
    });
    if (m_artwork) connect(m_artwork, &MediaArtwork::changed, this, &ProjectPanel::refresh);
    // Tiles are pixmaps in the theme's colours: redrawn in place, so the bin
    // keeps its selection and scroll position.
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, &ProjectPanel::refreshTiles);
}

void ProjectPanel::setSources(const QJsonArray &sources) {
    if (sources == m_sources) return;
    m_sources = sources;
    refresh();
}

void ProjectPanel::setMediaStatus(const QJsonObject &status) {
    QHash<QString, QString> badges;
    // A missing file outranks the rest: it cannot be downloaded or consolidated.
    for (const auto &[key, badge] : {std::pair{QStringLiteral("external"), kExternal},
                                     std::pair{QStringLiteral("pointers"), kNotDownloaded},
                                     std::pair{QStringLiteral("missing"), kOffline}})
        for (const QJsonValue &id : status.value(key).toArray()) badges.insert(id.toString(), badge);
    if (badges == m_badges) return;
    m_badges = badges;
    refresh();
}

QIcon ProjectPanel::tileFor(const QJsonObject &source) const {
    const QString id = source.value(QStringLiteral("id")).toString();
    return tile(m_artwork ? m_artwork->frames.value(id) : QImage(), m_artwork ? m_artwork->waves.value(id) : QImage(),
                !source.contains(QStringLiteral("width")), m_badges.value(id));
}

void ProjectPanel::refreshTiles() {
    // refresh() makes one item per source, in order.
    if (m_list->count() != m_sources.size()) {
        refresh();
        return;
    }
    for (int row = 0; row < m_list->count(); ++row) m_list->item(row)->setIcon(tileFor(m_sources.at(row).toObject()));
}

void ProjectPanel::refresh() {
    const QString current = m_list->currentItem() ? m_list->currentItem()->data(Qt::UserRole).toString() : QString();
    m_list->clear();
    for (const QJsonValue &value : m_sources) {
        const QJsonObject source = value.toObject();
        const QString id = source.value(QStringLiteral("id")).toString();
        const QString badge = m_badges.value(id);
        auto *item = new QListWidgetItem(tileFor(source),
                                         QStringLiteral("%1\n%2").arg(sourceName(source),
                                                                      prettyTime(source.value(QStringLiteral("duration")).toDouble())));
        item->setData(Qt::UserRole, id);
        QString tip = QStringLiteral("%1\nDouble-click to open in the Source monitor").arg(source.value(QStringLiteral("path")).toString());
        if (!badge.isEmpty()) tip += QStringLiteral("\n") + badgeTip(badge);
        item->setToolTip(tip);
        m_list->addItem(item);
        if (id == current) m_list->setCurrentItem(item);
    }
}

} // namespace kerf
