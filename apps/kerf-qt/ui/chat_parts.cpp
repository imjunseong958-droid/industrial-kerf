#include "chat_parts.h"

#include "components.h"
#include "design_tokens.h"
#include "icons.h"

#include <QEnterEvent>
#include <QFontMetricsF>
#include <QFontMetrics>
#include <QLocale>
#include <QPainter>
#include <QPainterPath>
#include <QRegularExpression>
#include <QTimer>
#include <QVariantAnimation>

#include <algorithm>
#include <cmath>
#include <limits>

namespace kerf {

using namespace tokens;

namespace chat {

namespace {

int frameRate(double fps) { return std::max(1, int(std::lround(std::max(1.0, fps)))); }

QString colorName(QRgb rgb) { return QColor::fromRgba(rgb).name(QColor::HexRgb); }

} // namespace

QString timecode(double seconds, double fps) {
    if (!std::isfinite(seconds)) return {};
    const int rate = frameRate(fps);
    const qint64 frames = std::llround(std::max(0.0, seconds) * std::max(1.0, fps));
    const qint64 f = frames % rate;
    qint64 whole = frames / rate;
    const qint64 s = whole % 60;
    whole /= 60;
    const qint64 m = whole % 60;
    const qint64 h = whole / 60;
    return QStringLiteral("%1:%2:%3:%4")
        .arg(h, 2, 10, QLatin1Char('0'))
        .arg(m, 2, 10, QLatin1Char('0'))
        .arg(s, 2, 10, QLatin1Char('0'))
        .arg(f, 2, 10, QLatin1Char('0'));
}

QString lengthText(double seconds, double fps) {
    const int rate = frameRate(fps);
    const qint64 frames = std::llround(std::max(0.0, seconds) * std::max(1.0, fps));
    const qint64 whole = frames / rate;
    const qint64 rest = frames % rate;
    if (whole == 0) return QStringLiteral("%1f").arg(rest);
    return QStringLiteral("%1s%2f").arg(whole).arg(rest, 2, 10, QLatin1Char('0'));
}

QString markText(double at, double duration, double fps) {
    if (!std::isfinite(at)) return {};
    const QString start = timecode(at, fps);
    if (!(duration > 0.0)) return start;
    return QStringLiteral("%1 · %2").arg(start, lengthText(duration, fps));
}

QString signedFrames(qint64 frames) {
    if (frames == 0) return QStringLiteral("0f");
    return QStringLiteral("%1%2f").arg(frames < 0 ? QStringLiteral("−") : QStringLiteral("+")).arg(std::abs(frames));
}

QString age(const QDateTime &then, const QDateTime &now) {
    if (!then.isValid()) return {};
    const qint64 seconds = then.secsTo(now);
    if (seconds < 60) return QStringLiteral("now");
    if (seconds < 3600) return QStringLiteral("%1 min").arg(seconds / 60);
    if (seconds < 86400) return QStringLiteral("%1 h").arg(seconds / 3600);
    const QDate day = then.toLocalTime().date();
    if (seconds < 7 * 86400) return QLocale::c().dayName(day.dayOfWeek(), QLocale::ShortFormat);
    if (day.year() == now.toLocalTime().date().year()) return QLocale::c().toString(day, QStringLiteral("d MMM"));
    return QLocale::c().toString(day, QStringLiteral("d MMM yyyy"));
}

QString ageTip(qint64 revision, const QDateTime &created) {
    if (!created.isValid()) return revision >= 0 ? QStringLiteral("rev %1").arg(revision) : QString();
    const QDateTime local = created.toLocalTime();
    const QString when = QLocale::c().toString(local, QStringLiteral("HH:mm, d MMM"));
    return revision >= 0 ? QStringLiteral("rev %1 · %2").arg(revision).arg(when) : when;
}

double parseTimecode(const QString &text, double fps) {
    static const QRegularExpression pattern(QStringLiteral("^(\\d{2}):(\\d{2}):(\\d{2})[:;](\\d{2})$"));
    const QRegularExpressionMatch match = pattern.match(text.trimmed());
    if (!match.hasMatch()) return std::numeric_limits<double>::quiet_NaN();
    // The inverse of timecode(): whole frames of the rounded rate, at fps.
    const qint64 whole = match.captured(1).toLongLong() * 3600 + match.captured(2).toLongLong() * 60 +
                         match.captured(3).toLongLong();
    const qint64 frames = whole * frameRate(fps) + match.captured(4).toLongLong();
    return double(frames) / std::max(1.0, fps);
}

// A word too long for any line a comment has, such as a file's name, may break anywhere: a QLabel
// breaks only at spaces, and cuts off the rest. Applied to text not yet escaped.
QString breakingLongWords(const QString &text) {
    constexpr int kLongWord = 24;
    static const QRegularExpression longWord(QStringLiteral("\\S{%1,}").arg(kLongWord));
    QString out;
    qsizetype from = 0;
    for (auto it = longWord.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        out += text.mid(from, match.capturedStart() - from).toHtmlEscaped();
        // Past each character, escaped alone so that no entity is broken.
        for (const QChar c : match.captured(0)) {
            out += QString(c).toHtmlEscaped();
            if (!c.isHighSurrogate()) out += QChar(0x200B);
        }
        from = match.capturedEnd();
    }
    return out + text.mid(from).toHtmlEscaped();
}

// Kerf writes its replies in Markdown: **bold** and `code` read as such, not as their marks.
QString withInlineMarks(const QString &text) {
    static const QRegularExpression mark(QStringLiteral("\\*\\*([^*\\n]+)\\*\\*|`([^`\\n]+)`"));
    QString out;
    qsizetype from = 0;
    for (auto it = mark.globalMatch(text); it.hasNext();) {
        const QRegularExpressionMatch match = it.next();
        out += breakingLongWords(text.mid(from, match.capturedStart() - from));
        if (match.capturedLength(1) > 0)
            out += QStringLiteral("<b>%1</b>").arg(breakingLongWords(match.captured(1)));
        else
            out += QStringLiteral("<span style=\"font-family:'%1','%2'; font-size:12px;\">%3</span>")
                       .arg(QString::fromLatin1(monoFamily), QString::fromLatin1(monoFallback),
                            breakingLongWords(match.captured(2)));
        from = match.capturedEnd();
    }
    return out + breakingLongWords(text.mid(from));
}

QString titleLine(const QString &body) {
    static const QRegularExpression bullet(QStringLiteral("^\\s*[-*] "));
    QString line = body.section(QLatin1Char('\n'), 0, 0);
    line.remove(bullet);
    return line.remove(QStringLiteral("**")).remove(QLatin1Char('`')).trimmed();
}

QString richBody(const QString &source, double fps) {
    // A Markdown list's "- " starts its line with a bullet.
    static const QRegularExpression bullet(QStringLiteral("^([ \\t]*)[-*] "), QRegularExpression::MultilineOption);
    QString body = source;
    body.replace(bullet, QStringLiteral("\\1\u2022 "));
    static const QRegularExpression token(QStringLiteral("(\\d{2}:\\d{2}:\\d{2}[:;]\\d{2})"));
    // The link colour is accent ink at the design's 85% over the card.
    const QString link = ui::mix(color(card), color(accentInk), 0.85).name(QColor::HexRgb);
    QString out;
    qsizetype from = 0;
    QRegularExpressionMatchIterator it = token.globalMatch(body);
    while (it.hasNext()) {
        const QRegularExpressionMatch match = it.next();
        out += withInlineMarks(body.mid(from, match.capturedStart() - from));
        const QString text = match.captured(0);
        const double seconds = parseTimecode(text, fps);
        out += QStringLiteral("<a href=\"tc:%1\" style=\"color:%2; text-decoration:none; font-family:'%3','%4'; "
                              "font-size:12px;\">%5</a>")
                   .arg(QString::number(seconds, 'g', 12), link, QString::fromLatin1(monoFamily),
                        QString::fromLatin1(monoFallback), text.toHtmlEscaped());
        from = match.capturedEnd();
    }
    out += withInlineMarks(body.mid(from));
    out.replace(QLatin1Char('\n'), QStringLiteral("<br>"));
    return QStringLiteral("<div style=\"line-height:20px;\">%1</div>").arg(out);
}

} // namespace chat

ChatExpander::ChatExpander(QWidget *parent) : QAbstractButton(parent) {
    setCheckable(true);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setFont(tokens::font(control));
    setFixedHeight(20);
    m_rotate = new QVariantAnimation(this);
    m_rotate->setEasingCurve(easing());
    connect(m_rotate, &QVariantAnimation::valueChanged, this, [this](const QVariant &value) {
        m_turn = value.toReal();
        update();
    });
    connect(this, &QAbstractButton::toggled, this, [this](bool on) {
        m_rotate->stop();
        const int ms = ui::motionDuration(motion::fade);
        if (ms <= 0 || !isVisible()) {
            m_turn = on ? 1 : 0;
            update();
            return;
        }
        m_rotate->setDuration(ms);
        m_rotate->setStartValue(m_turn);
        m_rotate->setEndValue(on ? 1.0 : 0.0);
        m_rotate->start();
    });
}

void ChatExpander::setOpenQuietly(bool open) {
    const QSignalBlocker blocker(this);
    setChecked(open);
    m_rotate->stop();
    m_turn = open ? 1 : 0;
    update();
}

void ChatExpander::setSmall(bool small) {
    m_small = small;
    setFont(tokens::font(small ? meta : control));
    setFixedHeight(small ? meta.line : 20);
    updateGeometry();
}

void ChatExpander::setChevron(bool chevron) {
    m_chevron = chevron;
    setCheckable(chevron);
    updateGeometry();
    update();
}

QSize ChatExpander::sizeHint() const {
    const int text = ui::snapWidth(QFontMetricsF(font()), this->text());
    return QSize(text + (m_chevron ? 4 + 14 : 0), m_small ? meta.line : 20);
}

void ChatExpander::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    const QColor ink = underMouse() ? color(tokens::ink) : color(ink2);
    p.setFont(font());
    p.setPen(ink);
    const QFontMetricsF metrics(font());
    p.drawText(QPointF(0, (height() - metrics.height()) / 2 + metrics.ascent()), text());
    if (m_chevron) {
        const qreal x = ui::snapWidth(metrics, text()) + 4;
        const QRectF box(x, (height() - 14) / 2.0, 14, 14);
        p.save();
        p.translate(box.center());
        p.rotate(180.0 * m_turn);
        p.translate(-box.center());
        icons::paint(p, QStringLiteral("g-chev"), box, ink, 14);
        p.restore();
    }
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), 2);
}

// ------------------------------------------------------------------ SuggestionButton

namespace {
// As the rows of the card over it are (the sidebar's): 34 tall, an 18 pt icon
// 12 in, the words 8 after it.
constexpr int kSuggestionHeight = 34;
constexpr int kSuggestionPad = 12;
constexpr int kSuggestionIcon = 18;
} // namespace

SuggestionButton::SuggestionButton(const QString &text, QWidget *parent) : QAbstractButton(parent), m_request(text) {
    setText(text);
    setFont(tokens::font(side));
    setFixedHeight(kSuggestionHeight);
    setCursor(Qt::PointingHandCursor);
    setFocusPolicy(Qt::TabFocus);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
}

QSize SuggestionButton::sizeHint() const {
    return QSize(kSuggestionPad + kSuggestionIcon + 8 + ui::snapWidth(QFontMetricsF(font()), text()) + kSuggestionPad, kSuggestionHeight);
}

QRect SuggestionButton::textRect() const {
    const int left = kSuggestionPad + kSuggestionIcon + 8;
    return QRect(left, 0, width() - left - kSuggestionPad, height());
}

void SuggestionButton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    if (underMouse() || isDown()) {
        p.setPen(Qt::NoPen);
        p.setBrush(color(raised));
        p.drawRoundedRect(QRectF(rect()), radius::control, radius::control);
    }
    icons::paint(p, QStringLiteral("g-cdr"), QRectF(kSuggestionPad, 0, kSuggestionIcon, height()), color(ink3), kSuggestionIcon);
    // Timecodes in the words read in mono ink2, as the welcome writes them.
    static const QRegularExpression tc(QStringLiteral("\\d{2}:\\d{2}:\\d{2}[:;]\\d{2}"));
    const QRect area = textRect();
    const QFont uiFont = font();
    const QFont mono = tokens::font(timecode);
    const QFontMetricsF uiMetrics(uiFont);
    qreal x = area.left();
    const qreal baseline = (height() - uiMetrics.height()) / 2 + uiMetrics.ascent();
    const QString words = text();
    qsizetype from = 0;
    const auto draw = [&](const QString &part, const QFont &face, const QColor &ink) {
        if (part.isEmpty() || x >= area.right()) return;
        const QFontMetricsF metrics(face);
        const QString shown = metrics.elidedText(part, Qt::ElideRight, area.right() - x);
        p.setFont(face);
        p.setPen(ink);
        p.drawText(QPointF(x, baseline), shown);
        x += metrics.horizontalAdvance(shown);
    };
    QRegularExpressionMatchIterator it = tc.globalMatch(words);
    while (it.hasNext()) {
        const QRegularExpressionMatch match = it.next();
        draw(words.mid(from, match.capturedStart() - from), uiFont, color(ink));
        draw(match.captured(0), mono, color(ink2));
        from = match.capturedEnd();
    }
    draw(words.mid(from), uiFont, color(ink));
    if (hasFocus()) ui::paintFocusRing(p, QRectF(rect()), radius::control);
}

// ------------------------------------------------------------------ DraftDots

DraftDots::DraftDots(QWidget *parent) : QWidget(parent) {
    setFixedSize(sizeHint());
    m_timer = new QTimer(this);
    m_timer->setInterval(320);
    connect(m_timer, &QTimer::timeout, this, [this] {
        m_phase = (m_phase + 1) % 3;
        update();
    });
}

void DraftDots::showEvent(QShowEvent *) {
    if (!ui::reducedMotion()) m_timer->start();
}

void DraftDots::hideEvent(QHideEvent *) { m_timer->stop(); }

void DraftDots::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    const bool moving = m_timer->isActive();
    for (int i = 0; i < 3; ++i) {
        QColor dot = color(ink3);
        if (moving && i != m_phase) dot.setAlphaF(0.45);
        p.setBrush(dot);
        p.drawEllipse(QRectF(2 + i * 6, (height() - 3) / 2.0, 3, 3));
    }
}

// ------------------------------------------------------------------ DraftSkeleton

DraftSkeleton::DraftSkeleton(QWidget *parent) : QWidget(parent) {
    setFixedHeight(8 + 8 + 8);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Fixed);
}

void DraftSkeleton::paintEvent(QPaintEvent *) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing);
    p.setPen(Qt::NoPen);
    p.setBrush(color(raised));
    p.drawRoundedRect(QRectF(0, 0, width() * 0.88, 8), radius::small, radius::small);
    p.drawRoundedRect(QRectF(0, 16, width() * 0.56, 8), radius::small, radius::small);
}

} // namespace kerf
