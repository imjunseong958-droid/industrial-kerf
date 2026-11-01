#include "project_panel.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"

#include <QAction>
#include <QEvent>
#include <QFontMetricsF>
#include <QJsonObject>
#include <QLineEdit>
#include <QListWidget>
#include <QPainter>
#include <QPainterPath>
#include <QStyledItemDelegate>
#include <QVBoxLayout>

#include <numeric>
#include <utility>

namespace kerf {

namespace {

constexpr QSize kThumbnail = ProjectPanel::kThumbnail;
constexpr int kTileRadius = 6;
// Under the thumbnail: the name (12/16), then the duration and the shape
// (11/14).
constexpr int kNameTop = 6;
constexpr int kNameHeight = 16;
constexpr int kFactsHeight = 14;
// Where the bin keeps a media's shape ("16:9", or "Audio") for its tile.
constexpr int kShapeRole = Qt::UserRole + 1;

const QString kOffline = QStringLiteral("Offline");
const QString kNotDownloaded = QStringLiteral("Not downloaded");
const QString kExternal = QStringLiteral("External");

// Where a picture of `size` sits fitted into a tile, centred, on whole
// pixels of a screen at `ratio`.
QRectF fittedRect(const QSize &size, qreal ratio) {
    const QSizeF fitted = QSizeF(size).scaled(QSizeF(kThumbnail), Qt::KeepAspectRatio);
    const QSizeF device(std::max(1.0, std::round(fitted.width() * ratio)), std::max(1.0, std::round(fitted.height() * ratio)));
    const QPointF at(std::round((kThumbnail.width() * ratio - device.width()) / 2.0),
                     std::round((kThumbnail.height() * ratio - device.height()) / 2.0));
    return QRectF(at / ratio, device / ratio);
}

// A thumbnail letterboxed onto a fixed tile, so the bin keeps a grid, with
// the media's badge in its corner. Drawn at the screen's pixel ratio, from
// a poster at its own shape: the bars are the tile's colour, not black
// baked into the picture. Sound is a teal tile with its waveform (or a
// waveform symbol until one arrives); a picture not yet drawn, or offline,
// is a quiet tile with an edge, so it never reads as a gap in the grid.
QIcon tile(const QImage &frame, const AudioPeaks &peaks, bool audio, const QString &badge, qreal ratio) {
    const EditorTheme &theme = editorTheme();
    QPixmap pixmap(QSize(int(std::ceil(kThumbnail.width() * ratio)), int(std::ceil(kThumbnail.height() * ratio))));
    pixmap.setDevicePixelRatio(ratio);
    QPainter painter(&pixmap);
    if (!frame.isNull() && badge != kOffline) {
        pixmap.fill(theme.thumbnail);
        // Averaged down to the device pixels it covers, once, rather than
        // sampled by the painter.
        const QRectF target = fittedRect(frame.size(), ratio);
        QImage scaled = frame.scaled((target.size() * ratio).toSize(), Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
        scaled.setDevicePixelRatio(ratio);
        painter.drawImage(target.topLeft(), scaled);
    } else if (audio && badge != kOffline) {
        pixmap.fill(theme.audioTile);
        if (!peaks.isEmpty()) {
            const QRectF wave(6, 12, kThumbnail.width() - 12, kThumbnail.height() - 24);
            painter.fillRect(QRectF(wave.left(), std::floor(wave.center().y()), wave.width(), 1),
                             mixColor(theme.audioTile, theme.audioTileGlyph, 0.3));
            drawWaveform(painter, wave, peaks, 0.0, peaks.duration(), mixColor(theme.audioTile, theme.audioTileGlyph, 0.55),
                         theme.audioTileGlyph);
        } else {
            drawWaveformSymbol(painter, QRectF(QPointF(), QSizeF(kThumbnail)).center(), 24, theme.audioTileGlyph);
        }
    } else {
        pixmap.fill(theme.tileEmpty);
        painter.setPen(theme.tileEmptyBorder);
        painter.drawRect(QRect(QPoint(), kThumbnail).adjusted(0, 0, -1, -1));
        // What the media is, as an 18-point symbol: SF Symbols' film, or a
        // waveform for sound. The badge and the name under the tile say the rest.
        if (audio) {
            drawWaveformSymbol(painter, QRectF(QPointF(), QSizeF(kThumbnail)).center(), 18, theme.tileEmptyText);
        } else {
            painter.save();
            painter.setRenderHint(QPainter::Antialiasing);
            painter.translate(kThumbnail.width() / 2.0 - 9, kThumbnail.height() / 2.0 - 9);
            painter.setPen(QPen(theme.tileEmptyText, 1.5, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            QPainterPath symbol;
            symbol.addRoundedRect(QRectF(1.5, 3, 15, 12), 2, 2);
            symbol.moveTo(5, 3); symbol.lineTo(5, 15);
            symbol.moveTo(13, 3); symbol.lineTo(13, 15);
            for (const qreal y : {7.0, 11.0}) {
                symbol.moveTo(1.5, y); symbol.lineTo(5, y);
                symbol.moveTo(13, y); symbol.lineTo(16.5, y);
            }
            painter.drawPath(symbol);
            painter.restore();
        }
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

// "16:9", "9:16", "1:1", "4:5", or the reduced ratio; "Audio" for sound.
QString shapeOf(const QJsonObject &source) {
    const int width = source.value(QStringLiteral("width")).toInt();
    const int height = source.value(QStringLiteral("height")).toInt();
    if (width <= 0) return QStringLiteral("Audio");
    if (height <= 0) return QString();
    const double ratio = double(width) / height;
    for (const auto &[w, h] : {std::pair{16, 9}, std::pair{9, 16}, std::pair{1, 1}, std::pair{4, 5}, std::pair{4, 3}, std::pair{21, 9}})
        if (std::abs(ratio - double(w) / h) < 0.01) return QStringLiteral("%1:%2").arg(w).arg(h);
    const int common = std::gcd(width, height);
    return QStringLiteral("%1:%2").arg(width / common).arg(height / common);
}

// One tile of the bin's grid, as the structure the owner chose draws it:
// the thumbnail with rounded corners, its name under it, then its duration
// on the left and its shape on the right in secondary ink. The chosen tile
// has a 2px accent ring round its thumbnail; no wash, no box.
class BinTileDelegate final : public QStyledItemDelegate {
public:
    using QStyledItemDelegate::QStyledItemDelegate;

    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override {
        return QSize(kThumbnail.width(), kThumbnail.height() + kNameTop + kNameHeight + kFactsHeight);
    }

    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        const EditorTheme &theme = editorTheme();
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        const QRect cell = option.rect;
        const QRectF thumb(cell.left() + (cell.width() - kThumbnail.width()) / 2.0, cell.top() + ProjectPanel::kTileInset,
                           kThumbnail.width(), kThumbnail.height());
        QPainterPath round;
        round.addRoundedRect(thumb, kTileRadius, kTileRadius);
        painter->save();
        painter->setClipPath(round);
        const QIcon icon = qvariant_cast<QIcon>(index.data(Qt::DecorationRole));
        painter->drawPixmap(thumb.toRect(), icon.pixmap(kThumbnail, painter->device()->devicePixelRatioF()));
        if (option.state & QStyle::State_MouseOver) painter->fillRect(thumb, theme.hover);
        painter->restore();
        if (option.state & QStyle::State_Selected) {
            painter->setPen(QPen(theme.accent, 2));
            painter->setBrush(Qt::NoBrush);
            painter->drawRoundedRect(thumb.adjusted(-1, -1, 1, 1), kTileRadius + 1, kTileRadius + 1);
        }
        const QStringList lines = index.data(Qt::DisplayRole).toString().split(QLatin1Char('\n'));
        QFont name = option.font;
        name.setPixelSize(12);
        painter->setFont(name);
        painter->setPen(theme.text);
        const QRectF nameRect(thumb.left(), thumb.bottom() + kNameTop, thumb.width(), kNameHeight);
        painter->drawText(nameRect, Qt::AlignLeft | Qt::AlignVCenter,
                          QFontMetrics(name).elidedText(lines.value(0), Qt::ElideMiddle, int(nameRect.width())));
        QFont facts = option.font;
        facts.setPixelSize(11);
        facts.setFeature(QFont::Tag("tnum"), 1);
        painter->setFont(facts);
        painter->setPen(theme.textSecondary);
        const QRectF factsRect(thumb.left(), nameRect.bottom(), thumb.width(), kFactsHeight);
        painter->drawText(factsRect, Qt::AlignLeft | Qt::AlignVCenter, lines.value(1));
        painter->drawText(factsRect, Qt::AlignRight | Qt::AlignVCenter, index.data(kShapeRole).toString());
        painter->restore();
    }
};

} // namespace

ProjectPanel::ProjectPanel(MediaArtwork *artwork, QWidget *parent) : QWidget(parent), m_artwork(artwork) {
    setObjectName(QStringLiteral("projectPanel"));
    setAttribute(Qt::WA_StyledBackground);
    auto *layout = new QVBoxLayout(this);
    // "Search media" over a grid of two columns of tiles, on the panel's
    // own surface, as the structure the owner chose.
    layout->setContentsMargins(0, 12, 0, 0);
    layout->setSpacing(8);
    m_search = new QLineEdit;
    m_search->setObjectName(QStringLiteral("binSearch"));
    m_search->setPlaceholderText(QStringLiteral("Search media"));
    m_search->setAccessibleName(QStringLiteral("Search media"));
    m_search->setClearButtonEnabled(true);
    m_search->addAction(searchGlyph(), QLineEdit::LeadingPosition);
    auto *searchRow = new QHBoxLayout;
    searchRow->setContentsMargins(kSidePadding, 0, kSidePadding, 0);
    searchRow->addWidget(m_search);
    layout->addLayout(searchRow);
    connect(m_search, &QLineEdit::textChanged, this, &ProjectPanel::applyFilter);
    connect(EditorAppearance::instance(), &EditorAppearance::changed, this, [this] {
        const QList<QAction *> actions = m_search->actions();
        if (!actions.isEmpty()) actions.constFirst()->setIcon(searchGlyph());
    });
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("projectBin"));
    m_list->setViewMode(QListView::IconMode);
    m_list->setIconSize(kThumbnail);
    m_list->setItemDelegate(new BinTileDelegate(m_list));
    m_list->setSpacing(0);
    m_list->setGridSize(QSize(kThumbnail.width() + kTileGap, kThumbnail.height() + kNameTop + kNameHeight + kFactsHeight + 12));
    m_list->setResizeMode(QListView::Adjust);
    m_list->setMovement(QListView::Static);
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setMouseTracking(true);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
    // The first column's thumbnails at the side padding: a grid cell is
    // kTileGap wider than its tile, which is centred in it.
    auto *grid = new QHBoxLayout;
    grid->setContentsMargins(kSidePadding - kTileGap / 2, 0, 0, 0);
    grid->addWidget(m_list);
    layout->addLayout(grid, 1);
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
    return tile(m_artwork ? m_artwork->frames.value(id) : QImage(), m_artwork ? m_artwork->peaks.value(id) : AudioPeaks(),
                !source.contains(QStringLiteral("width")), m_badges.value(id), devicePixelRatioF());
}

bool ProjectPanel::event(QEvent *event) {
    if (event->type() == QEvent::DevicePixelRatioChange) refreshTiles();
    return QWidget::event(event);
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
        item->setData(kShapeRole, shapeOf(source));
        QString tip = QStringLiteral("%1\nDouble-click to open in the Source monitor").arg(source.value(QStringLiteral("path")).toString());
        if (!badge.isEmpty()) tip += QStringLiteral("\n") + badgeTip(badge);
        item->setToolTip(tip);
        m_list->addItem(item);
        if (id == current) m_list->setCurrentItem(item);
    }
    applyFilter(m_search->text());
}

void ProjectPanel::applyFilter(const QString &text) {
    // By name, as typed, ignoring case.
    const QString wanted = text.trimmed();
    for (int row = 0; row < m_list->count(); ++row) {
        QListWidgetItem *item = m_list->item(row);
        item->setHidden(!wanted.isEmpty() && !item->text().section(QLatin1Char('\n'), 0, 0).contains(wanted, Qt::CaseInsensitive));
    }
}

QIcon ProjectPanel::searchGlyph() {
    // A magnifier, drawn in code (no SVG on Linux), in secondary ink.
    const qreal ratio = 2.0;
    QPixmap pixmap(QSize(16, 16) * ratio);
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(editorTheme().textSecondary, 1.5, Qt::SolidLine, Qt::RoundCap));
    painter.drawEllipse(QPointF(7, 7), 4.25, 4.25);
    painter.drawLine(QPointF(10.2, 10.2), QPointF(13.5, 13.5));
    return QIcon(pixmap);
}

} // namespace kerf
