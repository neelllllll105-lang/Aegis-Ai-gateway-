import { ImageResponse } from "next/og";

export const size = {
  width: 32,
  height: 32,
};
export const contentType = "image/png";

export default function Icon() {
  return new ImageResponse(
    (
      <div
        style={{
          width: "100%",
          height: "100%",
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          backgroundColor: "#F3EEDF",
          borderRadius: "8px",
          border: "1.5px solid #211C14",
        }}
      >
        <svg
          width="20"
          height="20"
          viewBox="0 0 24 24"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
        >
          <path
            d="M12 2.2L4.2 5.4V11.6C4.2 16.9 7.5 21.6 12 23C16.5 21.6 19.8 16.9 19.8 11.6V5.4L12 2.2Z"
            stroke="#A8341E"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <path
            d="M12 2.5L19.5 5.6V11.6C19.5 16.6 16.3 21.2 12 22.7V2.5Z"
            fill="#A8341E"
            fillOpacity="0.14"
          />
          <path
            d="M12 6.5L7.2 16.8H9.6L10.8 14.1H13.2L14.4 16.8H16.8L12 6.5Z"
            fill="#A8341E"
            fillOpacity="0.25"
            stroke="#A8341E"
            strokeWidth="1.3"
            strokeLinejoin="round"
          />
          <path d="M12 9.4L12.8 12.3H11.2L12 9.4Z" fill="#A8341E" />
          <circle cx="12" cy="19" r="1.15" fill="#A8341E" />
        </svg>
      </div>
    ),
    {
      ...size,
    }
  );
}
