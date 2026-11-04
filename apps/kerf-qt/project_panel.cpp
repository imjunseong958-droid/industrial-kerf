#include "project_panel.h"
#include "design_tokens.h"
#include "editor_common.h"
#include "editor_theme.h"
#include "media_artwork.h"
#include "ui/components.h"
#include "ui/icons.h"
#include "ui/tray_shell.h"

#include <QEvent>
#include <QFontMetricsF>
#include <QHBoxLayout>
#include <QKeyEvent>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QAction>
#include <QSignalBlocker>
#include <QStackedWidget>
#include <QStyledItemDelegate>
#include <QVBoxLayout>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>
#include <functional>
#include <utility>

namespace kerf {

using tokens::color;

namespace {

constexpr QSize kThumbnail = ProjectPanel::kThumbnail;
constexpr qreal kThumbRadius = tokens::radius::small;
// The media id of a bin item, and of an Index row.
constexpr int kMediaRole = Qt::UserRole;
// A bin item's duration badge ("9s20f").
constexpr int kBadgeRole = Qt::UserRole + 1;
// An Index row: 0 a header, 1 a row; its right-hand text; the time it seeks.
constexpr int kIndexKindRole = Qt::UserRole + 2;
constexpr int kIndexMetaRole = Qt::UserRole + 3;
constexpr int kIndexTimeRole = Qt::UserRole + 4;

const QString kOffline = QStringLiteral("Offline");
const QString kNotDownloaded = QStringLiteral("Not downloaded");
const QString kExternal = QStringLiteral("External");

QString badgeTip(const QString &badge) {
    if (badge == kOffline) return QStringLiteral("Offline: the file is missing. Replace Media… points this slot at another file.");
    if (badge == kNotDownloaded) return QStringLiteral("Not downloaded: a Git LFS file that a sync downloads.");
    if (badge == kExternal)
        return QStringLiteral("Outside the project folder, or ignored by Git there, so collaborators do not have it. "
                              "Consolidate Media copies it into media/.");
    return {};
}

// The mockup's 8 pt alpha checkerboard.
void paintCheckerboard(QPainter &painter, const QRectF &rect) {
    for (int y = 0; y < rect.height(); y += 8)
        for (int x = 0; x < rect.width(); x += 8)
            painter.fillRect(QRectF(rect.left() + x, rect.top() + y, 8, 8),
                             ((x + y) / 8) % 2 ? QColor(0x30, 0x30, 0x30) : QColor(0x26, 0x26, 0x26));
}

// A media's thumb (mockup .bi .th0): 72 x 38 on black. A picture is fitted
// at its own shape (black either side when it is narrower), over the
// checkerboard when it has alpha; sound is its waveform on raised; a picture
// not decoded yet is black with the film symbol; offline media says so. The
// duration badge sits at right 4, bottom 4; another status is a dot.
QIcon tile(const QImage &frame, const AudioPeaks &peaks, bool audio, bool alpha, const QString &badge,
           const QString &duration, qreal ratio) {
    QPixmap pixmap(QSize(int(std::ceil(kThumbnail.width() * ratio)), int(std::ceil(kThumbnail.height() * ratio))));
    pixmap.setDevicePixelRatio(ratio);
    pixmap.fill(Qt::transparent);
    QPainter painter(&pixmap);
    painter.setRenderHint(QPainter::Antialiasing);
    const QRectF box{QPointF(0, 0), QSizeF(kThumbnail)};
    QPainterPath round;
    round.addRoundedRect(box, kThumbRadius, kThumbRadius);
    painter.setClipPath(round);
    const bool offline = badge == kOffline;
    if (audio && !offline) {
        painter.fillRect(box, color(tokens::raised));
        if (!peaks.isEmpty())
            drawWaveform(painter, box, peaks, 0.0, peaks.duration(), color(tokens::overlay::waveformPeak),
                         color(tokens::overlay::waveformRms), peaks.displayGain());
        else
            icons::paint(painter, QStringLiteral("g-audio"), box, color(tokens::ink3), 16);
    } else {
        painter.fillRect(box, Qt::black);
        if (alpha && !offline) paintCheckerboard(painter, box);
        if (offline) {
            painter.setFont(tokens::font(tokens::meta));
            painter.setPen(color(tokens::neg));
            painter.drawText(box.adjusted(0, 0, 0, -12), Qt::AlignCenter, kOffline);
        } else if (!frame.isNull()) {
            // Averaged down to the device pixels it covers, once.
            const QSizeF fitted = QSizeF(frame.size()).scaled(box.size(), Qt::KeepAspectRatio);
            const QSize device(std::max(1, int(std::round(fitted.width() * ratio))), std::max(1, int(std::round(fitted.height() * ratio))));
            QImage scaled = frame.scaled(device, Qt::IgnoreAspectRatio, Qt::SmoothTransformation);
            scaled.setDevicePixelRatio(ratio);
            const QPointF at(std::round((box.width() * ratio - device.width()) / 2.0) / ratio,
                             std::round((box.height() * ratio - device.height()) / 2.0) / ratio);
            painter.drawImage(at, scaled);
        } else {
            icons::paint(painter, QStringLiteral("g-video"), box, color(tokens::dis), 16);
        }
    }
    if (!duration.isEmpty()) {
        // mono 11/12, white on 60% black, radius 4, pad 1 4, at right 4 / bottom 4.
        const QFont font = tokens::font(tokens::badge);
        painter.setFont(font);
        const int width = ui::snapWidth(QFontMetricsF(font), duration) + 8;
        const QRectF pill(box.right() - 4 - width, box.bottom() - 4 - (tokens::badge.line + 2), width, tokens::badge.line + 2);
        painter.setPen(Qt::NoPen);
        painter.setBrush(QColor(0, 0, 0, tokens::alpha(0.6)));
        painter.drawRoundedRect(pill, 4, 4);
        painter.setPen(Qt::white);
        painter.drawText(pill, Qt::AlignCenter, duration);
    }
    if (!badge.isEmpty() && !offline) {
        // Not downloaded (accent ink) or outside the project (ink3); the
        // tooltip names it.
        painter.setPen(Qt::NoPen);
        painter.setBrush(badge == kNotDownloaded ? color(tokens::accentInk) : color(tokens::ink3));
        painter.drawEllipse(QRectF(4, 4, 6, 6));
    }
    return QIcon(pixmap);
}

// A bin item: the thumb (with its 1 pt line, or a 1.5 pt ink ring when
// chosen) and the name under it; or, in the list, a 24 pt row.
class BinItemDelegate final : public QStyledItemDelegate {
public:
    explicit BinItemDelegate(QListWidget *view) : QStyledItemDelegate(view), m_view(view) {}
    bool list = false;

    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override {
        // A list row runs the width of the view.
        if (list) return QSize(std::max(1, m_view->viewport()->width()), ProjectPanel::kListRow);
        return QSize(kThumbnail.width() + ProjectPanel::kColumnGap, ProjectPanel::kItemHeight + ProjectPanel::kRowGap);
    }

    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        painter->save();
        painter->setRenderHint(QPainter::Antialiasing);
        const bool selected = option.state & QStyle::State_Selected;
        const QString name = index.data(Qt::DisplayRole).toString();
        if (list) {
            const QRectF row = QRectF(option.rect).adjusted(8, 0, -8, 0);
            if (selected || (option.state & QStyle::State_MouseOver)) {
                painter->setPen(Qt::NoPen);
                painter->setBrush(color(selected ? tokens::on : tokens::raised));
                painter->drawRoundedRect(row.adjusted(0, 2, 0, -2), tokens::radius::control, tokens::radius::control);
            }
            const QRectF text = row.adjusted(8, 0, -8, 0);
            const QString badge = index.data(kBadgeRole).toString();
            painter->setFont(tokens::font(tokens::monoSmall));
            painter->setPen(color(tokens::ink3));
            const int badgeWidth = ui::snapWidth(tokens::monoSmall, badge);
            painter->drawText(text, Qt::AlignRight | Qt::AlignVCenter, badge);
            painter->setFont(tokens::font(tokens::control));
            painter->setPen(color(selected ? tokens::ink : tokens::ink2));
            const QRectF nameRect = text.adjusted(0, 0, -(badgeWidth + 8), 0);
            painter->drawText(nameRect, Qt::AlignLeft | Qt::AlignVCenter,
                              QFontMetrics(painter->font()).elidedText(name, Qt::ElideRight, int(nameRect.width())));
            painter->restore();
            return;
        }
        const QRectF thumb(option.rect.left(), option.rect.top(), kThumbnail.width(), kThumbnail.height());
        const QIcon icon = qvariant_cast<QIcon>(index.data(Qt::DecorationRole));
        painter->drawPixmap(thumb.toRect(), icon.pixmap(kThumbnail, painter->device()->devicePixelRatioF()));
        if (selected) ui::paintInsetLine(*painter, thumb, kThumbRadius, color(tokens::ink), 1.5);
        else ui::paintInsetLine(*painter, thumb, kThumbRadius, color(tokens::line), 1);
        painter->setFont(tokens::font(tokens::meta));
        painter->setPen(color(selected ? tokens::ink : tokens::ink2));
        const QRectF nameRect(thumb.left(), thumb.bottom() + ProjectPanel::kNameTop, thumb.width(), tokens::meta.line);
        painter->drawText(nameRect, Qt::AlignLeft | Qt::AlignVCenter,
                          QFontMetrics(painter->font()).elidedText(name, Qt::ElideRight, int(nameRect.width())));
        painter->restore();
    }

private:
    QListWidget *m_view;
};

// An Index row (20 tall, pad 0 16): a header in meta ink3, or a row's name
// in ink with its time in mono ink3 on the right; hover raised.
class IndexDelegate final : public QStyledItemDelegate {
public:
    explicit IndexDelegate(QListWidget *view) : QStyledItemDelegate(view), m_view(view) {}
    QSize sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const override {
        return QSize(std::max(1, m_view->viewport()->width()), ProjectPanel::kIndexRow);
    }
    void paint(QPainter *painter, const QStyleOptionViewItem &option, const QModelIndex &index) const override {
        painter->save();
        const bool header = index.data(kIndexKindRole).toInt() == 0;
        const QRectF row(option.rect);
        if (!header && (option.state & QStyle::State_MouseOver)) painter->fillRect(row, color(tokens::raised));
        const QRectF text = row.adjusted(ProjectPanel::kSidePadding, 0, -ProjectPanel::kSidePadding, 0);
        if (header) {
            painter->setFont(tokens::font(tokens::meta));
            painter->setPen(color(tokens::ink3));
            painter->drawText(text, Qt::AlignLeft | Qt::AlignVCenter, index.data(Qt::DisplayRole).toString());
        } else {
            const QString meta = index.data(kIndexMetaRole).toString();
            painter->setFont(tokens::font(tokens::monoSmall));
            painter->setPen(color(tokens::ink3));
            painter->drawText(text, Qt::AlignRight | Qt::AlignVCenter, meta);
            painter->setFont(tokens::font(tokens::control));
            painter->setPen(color(tokens::ink));
            const QRectF name = text.adjusted(0, 0, -(ui::snapWidth(tokens::monoSmall, meta) + 8), 0);
            painter->drawText(name, Qt::AlignLeft | Qt::AlignVCenter,
                              QFontMetrics(painter->font()).elidedText(index.data(Qt::DisplayRole).toString(), Qt::ElideRight,
                                                                       int(name.width())));
        }
        painter->restore();
    }

private:
    QListWidget *m_view;
};

// A label in a type style and ink (the trays' helper).
QLabel *label(const QString &text, const tokens::TypeStyle &style, QRgb ink) {
    return shell::text(text, style, ink);
}

// The Roles header's "Edit roles…" (mockup .roles .rh .ghost-b): a ghost
// button 20 tall in UI 11 ink3; hover raised and ink.
class SmallGhost final : public ui::KitButton {
public:
    explicit SmallGhost(const QString &text, QWidget *parent = nullptr) : ui::KitButton(parent) {
        setText(text);
        setCursor(Qt::PointingHandCursor);
        setFixedSize(sizeHint());
    }
    QSize sizeHint() const override { return QSize(tokens::space::s8 * 2 + ui::snapWidth(tokens::meta, text()), tokens::height::h20); }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        if (hoverProgress() > 0) {
            QColor fill = color(tokens::raised);
            fill.setAlphaF(fill.alphaF() * hoverProgress());
            painter.setPen(Qt::NoPen);
            painter.setBrush(fill);
            painter.drawRoundedRect(QRectF(rect()), tokens::radius::control, tokens::radius::control);
        }
        painter.setFont(tokens::font(tokens::meta));
        painter.setPen(!isEnabled() ? color(tokens::dis) : ui::mix(color(tokens::ink3), color(tokens::ink), hoverProgress()));
        painter.drawText(rect(), Qt::AlignCenter, text());
    }
};

} // namespace

// The strip's segmented "Media N" | "Index" (mockup .seg #binTabs): the kit's
// segmented control, with the count after "Media" in ink3 at weight 400,
// which the kit's text segments cannot draw.
class BinTabs final : public QWidget {
public:
    std::function<void(int)> chosen;

    explicit BinTabs(QWidget *parent = nullptr) : QWidget(parent) {
        setObjectName(QStringLiteral("binTabs"));
        setMouseTracking(true);
        setCursor(Qt::PointingHandCursor);
        m_slide = new QVariantAnimation(this);
        m_slide->setEasingCurve(tokens::easing());
        connect(m_slide, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
            m_fill = value.toRectF();
            update();
        });
        relayout();
    }
    void setCount(int count) {
        if (count == m_count) return;
        m_count = count;
        relayout();
    }
    int count() const { return m_count; }
    int current() const { return m_current; }
    void setCurrent(int index) {
        if (index == m_current || index < 0 || index > 1) return;
        m_current = index;
        const QRectF target = segment(index);
        const int duration = ui::motionDuration(tokens::motion::segment);
        if (isVisible() && duration > 0) {
            m_slide->stop();
            m_slide->setDuration(duration);
            m_slide->setStartValue(m_fill);
            m_slide->setEndValue(target);
            m_slide->start();
        } else {
            m_fill = target;
            update();
        }
    }
    QRectF segment(int index) const { return index == 0 ? m_media : m_index; }
    QSize sizeHint() const override { return size(); }

protected:
    void paintEvent(QPaintEvent *) override {
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        painter.setBrush(color(tokens::raised));
        painter.drawRoundedRect(QRectF(rect()), tokens::radius::control, tokens::radius::control);
        painter.setBrush(color(tokens::on));
        painter.drawRoundedRect(m_fill, tokens::radius::small, tokens::radius::small);
        ui::paintInsetLine(painter, m_fill, tokens::radius::small, color(tokens::line2));
        ui::paintTopHighlight(painter, m_fill, tokens::radius::small, color(tokens::segmentHighlight));
        for (int index = 0; index < 2; ++index) {
            const bool on = index == m_current;
            const QRectF box = segment(index);
            const QColor ink = on || index == m_hover ? color(tokens::ink) : color(tokens::ink2);
            const QFont label = tokens::font(on ? tokens::controlStrong : tokens::control);
            if (index == 1) {
                painter.setFont(label);
                painter.setPen(ink);
                painter.drawText(box, Qt::AlignCenter, QStringLiteral("Index"));
                continue;
            }
            // "Media" then the count, as one centred run.
            const QString count = QStringLiteral(" %1").arg(m_count);
            const qreal nameWidth = QFontMetricsF(label).horizontalAdvance(QStringLiteral("Media"));
            const qreal countWidth = QFontMetricsF(tokens::font(tokens::control)).horizontalAdvance(count);
            const qreal left = box.left() + (box.width() - nameWidth - countWidth) / 2;
            painter.setFont(label);
            painter.setPen(ink);
            painter.drawText(QRectF(left, box.top(), nameWidth + 1, box.height()), Qt::AlignLeft | Qt::AlignVCenter, QStringLiteral("Media"));
            painter.setFont(tokens::font(tokens::control));
            painter.setPen(color(tokens::ink3));
            painter.drawText(QRectF(left + nameWidth, box.top(), countWidth + 1, box.height()), Qt::AlignLeft | Qt::AlignVCenter, count);
        }
    }
    void mouseMoveEvent(QMouseEvent *event) override {
        const int hover = m_media.contains(event->position()) ? 0 : m_index.contains(event->position()) ? 1 : -1;
        if (hover != m_hover) {
            m_hover = hover;
            update();
        }
    }
    void leaveEvent(QEvent *) override {
        m_hover = -1;
        update();
    }
    void mouseReleaseEvent(QMouseEvent *event) override {
        const int index = m_media.contains(event->position()) ? 0 : m_index.contains(event->position()) ? 1 : -1;
        if (index < 0) return;
        setCurrent(index);
        if (chosen) chosen(index);
    }

private:
    void relayout() {
        // Each segment as wide as its label at weight 500 (the count at 400),
        // pad 0 8, in a 24 track with 2 of padding.
        const int media = tokens::space::s8 + ui::snapWidth(tokens::controlStrong, QStringLiteral("Media")) +
                          ui::snapWidth(tokens::control, QStringLiteral(" %1").arg(m_count)) + tokens::space::s8;
        const int index = tokens::space::s8 + ui::snapWidth(tokens::controlStrong, QStringLiteral("Index")) + tokens::space::s8;
        m_media = QRectF(2, 2, media, tokens::height::h20);
        m_index = QRectF(2 + media, 2, index, tokens::height::h20);
        setFixedSize(2 + media + index + 2, tokens::height::h24);
        if (m_slide->state() != QAbstractAnimation::Running) m_fill = segment(m_current);
        updateGeometry();
        update();
    }

    QVariantAnimation *m_slide = nullptr;
    QRectF m_media;
    QRectF m_index;
    QRectF m_fill;
    int m_count = 0;
    int m_current = 0;
    int m_hover = -1;
};

ProjectPanel::ProjectPanel(MediaArtwork *artwork, QWidget *parent) : QWidget(parent), m_artwork(artwork) {
    setObjectName(QStringLiteral("projectPanel"));
    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    // The strip: 28 tall, pad 0 8 0 12, gap 4.
    auto *strip = new QWidget;
    strip->setObjectName(QStringLiteral("binStrip"));
    strip->setFixedHeight(tokens::layout::strip);
    auto *stripLayout = new QHBoxLayout(strip);
    stripLayout->setContentsMargins(tokens::space::s12, 0, tokens::space::s8, 0);
    stripLayout->setSpacing(tokens::space::s4);
    m_tabs = new BinTabs;
    m_tabs->chosen = [this](int index) { setTab(index == 0 ? Tab::Media : Tab::Index); };
    stripLayout->addWidget(m_tabs);
    stripLayout->addStretch(1);
    m_searchButton = new ui::IconButton(QStringLiteral("g-search"));
    m_searchButton->setObjectName(QStringLiteral("binSearchButton"));
    m_searchButton->setTip(QStringLiteral("Search media"));
    m_searchButton->setCheckable(true);
    m_viewButton = new ui::IconButton(QStringLiteral("g-list"));
    m_viewButton->setObjectName(QStringLiteral("binViewButton"));
    m_importButton = new ui::IconButton(QStringLiteral("g-import"));
    m_importButton->setObjectName(QStringLiteral("binImportButton"));
    m_importButton->setTip(QStringLiteral("Import media"), QStringLiteral("⌘I"));
    for (ui::IconButton *button : {m_searchButton, m_viewButton, m_importButton}) stripLayout->addWidget(button);
    layout->addWidget(strip);
    connect(m_searchButton, &QAbstractButton::clicked, this, [this](bool on) {
        if (on) focusSearch();
        else {
            m_search->clear();
            m_searchRow->hide();
        }
    });
    connect(m_viewButton, &QAbstractButton::clicked, this, [this] { setListView(!m_listView); });
    connect(m_importButton, &QAbstractButton::clicked, this, &ProjectPanel::importRequested);

    // Search media, shown by Search (⌘F) under the strip.
    m_searchRow = new QWidget;
    m_searchRow->setObjectName(QStringLiteral("binSearchRow"));
    m_searchRow->setFixedHeight(tokens::layout::strip);
    auto *searchLayout = new QHBoxLayout(m_searchRow);
    searchLayout->setContentsMargins(tokens::space::s12, 2, tokens::space::s8, 2);
    m_search = new QLineEdit;
    m_search->setObjectName(QStringLiteral("binSearch"));
    m_search->setPlaceholderText(QStringLiteral("Search media"));
    m_search->setAccessibleName(QStringLiteral("Search media"));
    m_search->setClearButtonEnabled(true);
    m_search->setFont(tokens::font(tokens::control));
    m_search->installEventFilter(this);
    searchLayout->addWidget(m_search);
    m_searchRow->hide();
    layout->addWidget(m_searchRow);
    connect(m_search, &QLineEdit::textChanged, this, &ProjectPanel::applyFilter);

    // The media page and the Index page, 192 tall on the well.
    m_pages = new QStackedWidget;
    m_pages->setObjectName(QStringLiteral("binPages"));
    m_pages->setFixedHeight(kGridHeight);
    m_well = new QWidget;
    m_well->setObjectName(QStringLiteral("binWell"));
    auto *wellLayout = new QVBoxLayout(m_well);
    wellLayout->setContentsMargins(kSidePadding, kTopPadding, 0, 0);
    wellLayout->setSpacing(0);
    m_list = new QListWidget;
    m_list->setObjectName(QStringLiteral("projectBin"));
    m_list->setItemDelegate(new BinItemDelegate(m_list));
    m_list->setFrameShape(QFrame::NoFrame);
    m_list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setMouseTracking(true);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
    m_list->setStyleSheet(QStringLiteral("QListWidget { background: transparent; border: none; outline: none; }"));
    m_list->viewport()->setAutoFillBackground(false);
    wellLayout->addWidget(m_list);
    m_pages->addWidget(m_well);
    auto *indexPage = new QWidget;
    indexPage->setObjectName(QStringLiteral("binIndexPage"));
    auto *indexLayout = new QVBoxLayout(indexPage);
    indexLayout->setContentsMargins(0, tokens::space::s4, 0, 0);
    indexLayout->setSpacing(0);
    m_index = new QListWidget;
    m_index->setObjectName(QStringLiteral("binIndex"));
    m_index->setItemDelegate(new IndexDelegate(m_index));
    m_index->setFrameShape(QFrame::NoFrame);
    m_index->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_index->setVerticalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_index->setMouseTracking(true);
    m_index->setSelectionMode(QAbstractItemView::NoSelection);
    m_index->setStyleSheet(QStringLiteral("QListWidget { background: transparent; border: none; outline: none; }"));
    m_index->viewport()->setAutoFillBackground(false);
    indexLayout->addWidget(m_index);
    m_indexEmpty = label(QString(), tokens::meta, tokens::ink3);
    m_indexEmpty->setObjectName(QStringLiteral("binIndexEmpty"));
    m_indexEmpty->setAlignment(Qt::AlignCenter);
    m_indexEmpty->setWordWrap(true);
    m_indexEmpty->setContentsMargins(kSidePadding, 0, kSidePadding, 0);
    indexLayout->addWidget(m_indexEmpty, 1);
    m_pages->addWidget(indexPage);
    layout->addWidget(m_pages);
    connect(m_index, &QListWidget::itemClicked, this, [this](QListWidgetItem *item) {
        if (item->data(kIndexKindRole).toInt() == 1)
            emit seekRequested(item->data(kMediaRole).toString(), item->data(kIndexTimeRole).toDouble());
    });

    // Roles: the rest of the band, scrolling when there are more than fit.
    auto *roles = new ui::FadeScrollArea;
    roles->setObjectName(QStringLiteral("binRoles"));
    roles->setFadeColor(color(tokens::chrome));
    roles->setStyleSheet(QStringLiteral("QScrollArea { background: transparent; border: none; }"));
    roles->viewport()->setAutoFillBackground(false);
    m_roleBox = new QWidget;
    m_roleBox->setObjectName(QStringLiteral("binRoleBox"));
    m_roleBox->setAutoFillBackground(false);
    auto *roleLayout = new QVBoxLayout(m_roleBox);
    roleLayout->setContentsMargins(0, 0, 0, tokens::space::s4);
    roleLayout->setSpacing(0);
    auto *header = new QWidget;
    header->setFixedHeight(kRoleRow);
    auto *headerLayout = new QHBoxLayout(header);
    headerLayout->setContentsMargins(kSidePadding, 0, tokens::space::s8, 0);
    headerLayout->setSpacing(0);
    headerLayout->addWidget(label(QStringLiteral("Roles"), tokens::controlStrong, tokens::ink));
    headerLayout->addStretch(1);
    auto *edit = new SmallGhost(QStringLiteral("Edit roles…"));
    edit->setObjectName(QStringLiteral("binEditRoles"));
    edit->setTip(QStringLiteral("Assign a role to each track"));
    m_editRoles = edit;
    headerLayout->addWidget(edit);
    connect(edit, &QAbstractButton::clicked, this, &ProjectPanel::editRolesRequested);
    roleLayout->addWidget(header);
    m_roleRows = new QVBoxLayout;
    m_roleRows->setContentsMargins(0, 0, 0, 0);
    m_roleRows->setSpacing(0);
    roleLayout->addLayout(m_roleRows);
    roleLayout->addStretch(1);
    roles->setWidget(m_roleBox);
    layout->addWidget(roles, 1);

    connect(m_list, &QListWidget::itemActivated, this, [this](QListWidgetItem *item) {
        emit openRequested(item->data(kMediaRole).toString());
    });
    connect(m_list, &QListWidget::currentItemChanged, this, [this] {
        emit currentMediaChanged(currentMedia());
        if (m_tab == Tab::Index) {
            rebuildIndex();
            requestIndex();
        }
    });
    connect(m_list, &QListWidget::customContextMenuRequested, this, [this](const QPoint &at) {
        QListWidgetItem *item = m_list->itemAt(at);
        if (item) m_list->setCurrentItem(item);
        emit menuRequested(item ? item->data(kMediaRole).toString() : QString(), m_list->viewport()->mapToGlobal(at));
    });
    if (m_artwork) connect(m_artwork, &MediaArtwork::changed, this, &ProjectPanel::refreshTiles);

    // ⌘F and ⌘⌥2 are the registry's bin.search and bin.toggleView
    // (setCommandActions).

    setListView(false);
    rebuildIndex();
    rebuildRoles();
}

bool ProjectPanel::event(QEvent *event) {
    if (event->type() == QEvent::DevicePixelRatioChange) refreshTiles();
    return QWidget::event(event);
}

bool ProjectPanel::eventFilter(QObject *watched, QEvent *event) {
    // Esc in the search field clears it and puts it away.
    if (watched == m_search && event->type() == QEvent::KeyPress && static_cast<QKeyEvent *>(event)->key() == Qt::Key_Escape) {
        m_search->clear();
        m_searchRow->hide();
        m_searchButton->setOn(false);
        m_list->setFocus(Qt::OtherFocusReason);
        update();
        return true;
    }
    return QWidget::eventFilter(watched, event);
}

void ProjectPanel::paintEvent(QPaintEvent *) {
    QPainter painter(this);
    painter.fillRect(rect(), color(tokens::chrome));
    const QColor line = color(tokens::line);
    // The strip's top and bottom lines; the pages on the well; Roles' top line.
    painter.fillRect(QRect(0, 0, width(), 1), line);
    painter.fillRect(QRect(0, tokens::layout::strip - 1, width(), 1), line);
    const QRect pages(m_pages->mapTo(this, QPoint()), m_pages->size());
    painter.fillRect(pages, color(tokens::well));
    if (m_searchRow->isVisible()) painter.fillRect(QRect(0, m_searchRow->geometry().bottom(), width(), 1), line);
    painter.fillRect(QRect(0, pages.bottom() + 1, width(), 1), line);
}

void ProjectPanel::setSources(const QJsonArray &sources) {
    if (sources == m_sources) return;
    // An index belongs to the file it was made of: a relinked or removed
    // media's goes.
    QHash<QString, QString> paths;
    for (const QJsonValue &value : sources)
        paths.insert(value.toObject().value(QStringLiteral("id")).toString(), value.toObject().value(QStringLiteral("path")).toString());
    for (const QJsonValue &value : std::as_const(m_sources)) {
        const QJsonObject old = value.toObject();
        const QString id = old.value(QStringLiteral("id")).toString();
        if (!paths.contains(id) || paths.value(id) != old.value(QStringLiteral("path")).toString()) {
            m_indexes.remove(id);
            m_indexErrors.remove(id);
            m_indexAsked.removeAll(id);
        }
    }
    m_sources = sources;
    m_tabs->setCount(int(sources.size()));
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

void ProjectPanel::setFps(double fps) {
    fps = fps > 0 ? fps : 24.0;
    if (fps == m_fps) return;
    m_fps = fps;
    refresh();
}

void ProjectPanel::setMediaInfo(const QJsonObject &media) {
    bool changed = false;
    for (auto it = media.begin(); it != media.end(); ++it) {
        const QJsonObject info = it.value().toObject();
        if (m_info.value(it.key()) == info) continue;
        m_info.insert(it.key(), info);
        changed = true;
    }
    if (changed) refreshTiles();
}

void ProjectPanel::setMediaIndex(const QJsonObject &index) {
    const QString media = index.value(QStringLiteral("media")).toString();
    if (media.isEmpty()) return;
    m_indexes.insert(media, index);
    m_indexErrors.remove(media);
    m_indexAsked.removeAll(media);
    if (media == currentMedia()) rebuildIndex();
}

void ProjectPanel::setMediaIndexError(const QString &media, const QString &message) {
    m_indexAsked.removeAll(media);
    m_indexErrors.insert(media, message);
    if (media.isEmpty() || media == currentMedia()) rebuildIndex();
}

QJsonObject ProjectPanel::sourceFor(const QString &mediaId) const {
    for (const QJsonValue &value : m_sources)
        if (value.toObject().value(QStringLiteral("id")).toString() == mediaId) return value.toObject();
    return {};
}

QIcon ProjectPanel::tileFor(const QJsonObject &source) const {
    const QString id = source.value(QStringLiteral("id")).toString();
    const bool audio = !source.contains(QStringLiteral("width"));
    const bool alpha = pixelFormatHasAlpha(m_info.value(id).value(QStringLiteral("pixFmt")).toString());
    return tile(m_artwork ? m_artwork->frames.value(id) : QImage(), m_artwork ? m_artwork->peaks.value(id) : AudioPeaks(), audio,
                alpha, m_badges.value(id), badgeText(source.value(QStringLiteral("duration")).toDouble(), m_fps), devicePixelRatioF());
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
    const QString current = currentMedia();
    const QSignalBlocker blocker(m_list);
    m_list->clear();
    const QHash<QString, QString> names = sourceNames(m_sources);
    for (const QJsonValue &value : std::as_const(m_sources)) {
        const QJsonObject source = value.toObject();
        const QString id = source.value(QStringLiteral("id")).toString();
        const QString badge = m_badges.value(id);
        const double duration = source.value(QStringLiteral("duration")).toDouble();
        auto *item = new QListWidgetItem(tileFor(source), names.value(id));
        item->setData(kMediaRole, id);
        item->setData(kBadgeRole, badgeText(duration, m_fps));
        // The name and its length as a timecode (mockup data-tip, data-k).
        QStringList tip{QStringLiteral("%1 · %2").arg(names.value(id), timelineTimecode(duration, m_fps)),
                        source.value(QStringLiteral("path")).toString(), QStringLiteral("Double-click to open in the Source viewer")};
        if (!badge.isEmpty()) tip << badgeTip(badge);
        item->setToolTip(tip.join(QLatin1Char('\n')));
        m_list->addItem(item);
        if (id == current) m_list->setCurrentItem(item);
    }
    applyFilter(m_search->text());
    if (current != currentMedia()) {
        emit currentMediaChanged(currentMedia());
        if (m_tab == Tab::Index) requestIndex();
    }
    rebuildIndex();
}

void ProjectPanel::applyFilter(const QString &text) {
    // By name, as typed, ignoring case.
    const QString wanted = text.trimmed();
    for (int row = 0; row < m_list->count(); ++row) {
        QListWidgetItem *item = m_list->item(row);
        item->setHidden(!wanted.isEmpty() && !item->text().contains(wanted, Qt::CaseInsensitive));
    }
}

QString ProjectPanel::currentMedia() const {
    const QListWidgetItem *item = m_list->currentItem();
    return item ? item->data(kMediaRole).toString() : QString();
}

void ProjectPanel::setCurrentMedia(const QString &mediaId) {
    for (int row = 0; row < m_list->count(); ++row)
        if (m_list->item(row)->data(kMediaRole).toString() == mediaId) {
            m_list->setCurrentRow(row);
            return;
        }
}

int ProjectPanel::mediaCount() const {
    return m_tabs->count();
}

QAbstractButton *ProjectPanel::importButton() const {
    return m_importButton;
}

void ProjectPanel::focusSearch() {
    m_searchRow->show();
    m_searchButton->setOn(true);
    if (m_tab != Tab::Media) setTab(Tab::Media);
    m_search->setFocus(Qt::ShortcutFocusReason);
    m_search->selectAll();
    update();
}

void ProjectPanel::setTab(Tab tab) {
    m_tabs->setCurrent(tab == Tab::Media ? 0 : 1);
    if (tab == m_tab) return;
    m_tab = tab;
    m_pages->setCurrentIndex(tab == Tab::Media ? 0 : 1);
    if (tab == Tab::Index) {
        rebuildIndex();
        requestIndex();
    }
}

void ProjectPanel::setListView(bool list) {
    m_listView = list;
    auto *delegate = static_cast<BinItemDelegate *>(m_list->itemDelegate());
    delegate->list = list;
    if (list) {
        m_list->setViewMode(QListView::ListMode);
        m_list->setGridSize(QSize());
        m_list->setUniformItemSizes(true);
        m_well->layout()->setContentsMargins(0, tokens::space::s4, 0, 0);
    } else {
        m_list->setViewMode(QListView::IconMode);
        m_list->setMovement(QListView::Static);
        m_list->setResizeMode(QListView::Adjust);
        m_list->setFlow(QListView::LeftToRight);
        m_list->setWrapping(true);
        m_list->setSpacing(0);
        m_list->setUniformItemSizes(true);
        m_list->setGridSize(QSize(kThumbnail.width() + kColumnGap, kItemHeight + kRowGap));
        m_list->setIconSize(kThumbnail);
        m_well->layout()->setContentsMargins(kSidePadding, kTopPadding, 0, 0);
    }
    m_list->setDragEnabled(false);
    m_list->doItemsLayout();
    updateViewButton();
}

void ProjectPanel::updateViewButton() {
    // The button offers the other view (mockup gridBtn).
    m_viewButton->setIconName(m_listView ? QStringLiteral("g-grid") : QStringLiteral("g-list"));
    m_viewButton->setTip(m_listView ? QStringLiteral("Grid view") : QStringLiteral("List view"),
                         m_viewAction ? m_viewAction->shortcut().toString(QKeySequence::NativeText) : QString());
}

void ProjectPanel::setCommandActions(QAction *search, QAction *toggleView) {
    for (QAction *old : {m_searchAction.data(), m_viewAction.data()})
        if (old) {
            removeAction(old);
            old->disconnect(this);
        }
    m_searchAction = search;
    m_viewAction = toggleView;
    const auto follow = [this] {
        m_searchButton->setTip(QStringLiteral("Search media"),
                               m_searchAction ? m_searchAction->shortcut().toString(QKeySequence::NativeText) : QString());
        updateViewButton();
    };
    // Search while the bin has the focus (the comments panel has its own
    // ⌘F); the view while the bin is on screen.
    if (search) {
        search->setShortcutContext(Qt::WidgetWithChildrenShortcut);
        addAction(search);
        connect(search, &QAction::changed, this, follow);
    }
    if (toggleView) {
        toggleView->setShortcutContext(Qt::WindowShortcut);
        addAction(toggleView);
        connect(toggleView, &QAction::changed, this, follow);
    }
    follow();
}

void ProjectPanel::requestIndex() {
    const QString media = currentMedia();
    if (media.isEmpty() || m_indexes.contains(media) || m_indexErrors.contains(media) || m_indexAsked.contains(media)) return;
    m_indexAsked << media;
    emit indexWanted(media);
    rebuildIndex();
}

void ProjectPanel::rebuildIndex() {
    m_index->clear();
    const QString media = currentMedia();
    const QJsonObject source = sourceFor(media);
    const QString name = sourceNames(m_sources).value(media);
    const auto empty = [this](const QString &text) {
        m_index->hide();
        m_indexEmpty->setText(text);
        m_indexEmpty->show();
    };
    if (media.isEmpty()) return empty(m_sources.isEmpty() ? QStringLiteral("Import media to index it")
                                                          : QStringLiteral("Choose media in the bin to see its shots"));
    if (m_indexErrors.contains(media)) return empty(m_indexErrors.value(media));
    if (!m_indexes.contains(media)) return empty(QStringLiteral("Indexing %1…").arg(name));
    const QJsonObject index = m_indexes.value(media);
    // Times are the media's own, counted at its rate.
    const double fps = source.value(QStringLiteral("fps")).toDouble(m_fps) > 0 ? source.value(QStringLiteral("fps")).toDouble(m_fps) : m_fps;
    const auto tc = [fps](double seconds) { return timelineTimecode(seconds, fps); };
    const auto addHeader = [this](const QString &text) {
        auto *item = new QListWidgetItem(text);
        item->setData(kIndexKindRole, 0);
        item->setFlags(Qt::ItemIsEnabled);
        m_index->addItem(item);
    };
    const auto addRow = [this, &media](const QString &text, const QString &meta, double at) {
        auto *item = new QListWidgetItem(text);
        item->setData(kIndexKindRole, 1);
        item->setData(kIndexMetaRole, meta);
        item->setData(kIndexTimeRole, at);
        item->setData(kMediaRole, media);
        item->setToolTip(QStringLiteral("Show in the Source viewer at %1").arg(meta.section(QStringLiteral("–"), 0, 0)));
        m_index->addItem(item);
    };
    // Shots: the engine's ranges, else the spans between its cuts.
    QList<std::pair<double, double>> shots;
    for (const QJsonValue &value : index.value(QStringLiteral("ranges")).toArray()) {
        const QJsonObject range = value.toObject();
        shots.append({range.value(QStringLiteral("start")).toDouble(), range.value(QStringLiteral("end")).toDouble()});
    }
    if (shots.isEmpty()) {
        QList<double> cuts;
        for (const QJsonValue &cut : index.value(QStringLiteral("cuts")).toArray()) cuts << cut.toDouble();
        std::sort(cuts.begin(), cuts.end());
        const double duration = source.value(QStringLiteral("duration")).toDouble();
        double start = 0.0;
        for (const double cut : std::as_const(cuts)) {
            if (cut > start) shots.append({start, cut});
            start = cut;
        }
        if (!cuts.isEmpty() && duration > start) shots.append({start, duration});
    }
    if (!shots.isEmpty()) {
        addHeader(shots.size() == 1 ? QStringLiteral("1 shot") : QStringLiteral("%1 shots").arg(shots.size()));
        for (int shot = 0; shot < shots.size(); ++shot)
            addRow(QStringLiteral("Shot %1").arg(shot + 1), QStringLiteral("%1–%2").arg(tc(shots.at(shot).first), tc(shots.at(shot).second)),
                   shots.at(shot).first);
    }
    const QJsonArray silences = index.value(QStringLiteral("silences")).toArray();
    if (!silences.isEmpty()) {
        addHeader(silences.size() == 1 ? QStringLiteral("1 silence") : QStringLiteral("%1 silences").arg(silences.size()));
        int number = 0;
        for (const QJsonValue &value : silences) {
            const QJsonArray span = value.toArray();
            addRow(QStringLiteral("Silence %1").arg(++number), QStringLiteral("%1–%2").arg(tc(span.at(0).toDouble()), tc(span.at(1).toDouble())),
                   span.at(0).toDouble());
        }
    }
    const QJsonArray loudest = index.value(QStringLiteral("loudest")).toArray();
    if (!loudest.isEmpty()) {
        addHeader(QStringLiteral("Loudest moments"));
        int number = 0;
        for (const QJsonValue &value : loudest) addRow(QStringLiteral("Loudest %1").arg(++number), tc(value.toDouble()), value.toDouble());
    }
    if (m_index->count() == 0) return empty(QStringLiteral("%1 has no cuts, silences or loud moments").arg(name));
    m_indexEmpty->hide();
    m_index->show();
}

QList<ProjectPanel::RoleGroup> ProjectPanel::roleGroups(const QJsonArray &tracks) {
    QList<RoleGroup> groups;
    for (const QJsonValue &value : tracks) {
        const QJsonObject track = value.toObject();
        const QString kind = track.value(QStringLiteral("type")).toString();
        const QString role = track.value(QStringLiteral("role")).toString().trimmed();
        const QString key = role.isEmpty() ? (kind == QStringLiteral("subtitle") ? QStringLiteral("captions") : kind) : role.toLower();
        auto group = std::find_if(groups.begin(), groups.end(), [&key](const RoleGroup &g) { return g.key == key; });
        if (group == groups.end()) {
            groups.append(RoleGroup{key, roleName(role, kind), {}, true});
            group = std::prev(groups.end());
        }
        group->tracks << track.value(QStringLiteral("id")).toString();
        group->enabled = group->enabled && track.value(QStringLiteral("enabled")).toBool(true);
    }
    return groups;
}

QString ProjectPanel::roleName(const QString &role, const QString &kind) {
    QString name = role.trimmed();
    if (name.isEmpty()) name = kind == QStringLiteral("subtitle") ? QStringLiteral("captions") : kind;
    if (name.isEmpty()) return QStringLiteral("Other");
    return name.left(1).toUpper() + name.mid(1);
}

QString ProjectPanel::badgeText(double seconds, double fps) {
    const double rate = fps > 0 ? fps : 24.0;
    const int perSecond = std::max(1, int(std::lround(rate)));
    const long long frames = std::llround(std::max(0.0, seconds) * rate);
    const long long whole = frames / perSecond;
    if (whole < 60) return QStringLiteral("%1s%2f").arg(whole).arg(frames % perSecond, 2, 10, QLatin1Char('0'));
    if (whole < 3600) return QStringLiteral("%1m%2s").arg(whole / 60).arg(whole % 60, 2, 10, QLatin1Char('0'));
    return QStringLiteral("%1h%2m").arg(whole / 3600).arg((whole / 60) % 60, 2, 10, QLatin1Char('0'));
}

bool ProjectPanel::pixelFormatHasAlpha(const QString &pixFmt) {
    const QString format = pixFmt.toLower();
    if (format.isEmpty()) return false;
    // FFmpeg's names: yuva*, rgba/bgra/argb/abgr*, ya8/ya16, gbrap*, pal8 can,
    // but rarely does; the a-carrying ones only.
    return format.startsWith(QStringLiteral("yuva")) || format.startsWith(QStringLiteral("ya")) ||
           format.startsWith(QStringLiteral("gbrap")) || format.contains(QStringLiteral("rgba")) ||
           format.contains(QStringLiteral("bgra")) || format.contains(QStringLiteral("argb")) || format.contains(QStringLiteral("abgr")) ||
           format.startsWith(QStringLiteral("ayuv"));
}

void ProjectPanel::setTracks(const QJsonArray &tracks) {
    m_tracks = tracks;
    const QList<RoleGroup> groups = roleGroups(tracks);
    const auto same = [](const QList<RoleGroup> &a, const QList<RoleGroup> &b) {
        if (a.size() != b.size()) return false;
        for (int i = 0; i < a.size(); ++i)
            if (a[i].key != b[i].key || a[i].name != b[i].name || a[i].tracks != b[i].tracks || a[i].enabled != b[i].enabled) return false;
        return true;
    };
    if (same(groups, m_roles) && m_roleChecks.size() == groups.size()) {
        // A click flips its box before kerfd answers; a refused or unchanged
        // edit leaves the tracks as they were, and the boxes say so again.
        for (int i = 0; i < m_roleChecks.size(); ++i) {
            const QSignalBlocker blocker(m_roleChecks[i]);
            m_roleChecks[i]->setChecked(m_roles[i].enabled);
        }
        return;
    }
    m_roles = groups;
    rebuildRoles();
}

void ProjectPanel::rebuildRoles() {
    while (QLayoutItem *item = m_roleRows->takeAt(0)) {
        if (QWidget *widget = item->widget()) widget->deleteLater();
        delete item;
    }
    m_roleChecks.clear();
    for (const RoleGroup &group : std::as_const(m_roles)) {
        auto *row = new QWidget;
        row->setObjectName(QStringLiteral("binRole_%1").arg(group.key));
        row->setFixedHeight(kRoleRow);
        auto *rowLayout = new QHBoxLayout(row);
        rowLayout->setContentsMargins(kSidePadding, 0, kSidePadding, 0);
        rowLayout->setSpacing(tokens::space::s8);
        auto *check = new ui::CheckBox;
        check->setObjectName(QStringLiteral("binRoleCheck_%1").arg(group.key));
        check->setChecked(group.enabled);
        check->setTip(group.enabled ? QStringLiteral("Turn off %1").arg(group.tracks.join(QLatin1Char(' ')))
                                    : QStringLiteral("Turn on %1").arg(group.tracks.join(QLatin1Char(' '))));
        const QStringList tracks = group.tracks;
        connect(check, &QAbstractButton::clicked, this, [this, tracks](bool on) { emit rolesToggled(tracks, on); });
        rowLayout->addWidget(check);
        rowLayout->addWidget(label(group.name, tokens::control, tokens::ink));
        rowLayout->addStretch(1);
        rowLayout->addWidget(label(group.tracks.join(QLatin1Char(' ')), tokens::monoSmall, tokens::ink3));
        m_roleRows->addWidget(row);
        m_roleChecks << check;
    }
}

} // namespace kerf
